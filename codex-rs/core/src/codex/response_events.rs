use super::*;

pub(super) struct StreamingResponseState {
    needs_follow_up: bool,
    last_agent_message: Option<String>,
    active_agent_message_item: Option<TurnItem>,
    active_reasoning_item: Option<TurnItem>,
    active_web_search_item: Option<TurnItem>,
    should_emit_turn_diff: bool,
    assistant_message_stream_parsers: AssistantMessageStreamParsers,
    plan_mode_state: Option<PlanModeStreamState>,
}

impl StreamingResponseState {
    pub(super) fn new(plan_mode: bool, turn_id: &str) -> Self {
        Self {
            needs_follow_up: false,
            last_agent_message: None,
            active_agent_message_item: None,
            active_reasoning_item: None,
            active_web_search_item: None,
            should_emit_turn_diff: false,
            assistant_message_stream_parsers: AssistantMessageStreamParsers::new(plan_mode),
            plan_mode_state: plan_mode.then(|| PlanModeStreamState::new(turn_id)),
        }
    }

    pub(super) async fn handle_output_item_done(
        &mut self,
        sess: &Arc<Session>,
        turn_context: &Arc<TurnContext>,
        tool_runtime: &ToolCallRuntime,
        cancellation_token: &CancellationToken,
        handle_responses: tracing::Span,
        item: ResponseItem,
    ) -> CodexResult<Option<BoxFuture<'static, CodexResult<ResponseInputItem>>>> {
        let previously_active_item = match &item {
            ResponseItem::Message { role, .. } if role == "assistant" => {
                self.active_agent_message_item.take()
            }
            ResponseItem::Reasoning { .. } => self.active_reasoning_item.take(),
            ResponseItem::WebSearchCall { .. } => self.active_web_search_item.take(),
            _ => None,
        };
        if let Some(previous) = previously_active_item.as_ref()
            && matches!(previous, TurnItem::AgentMessage(_))
        {
            let item_id = previous.id();
            flush_assistant_text_segments_for_item(
                sess,
                turn_context,
                self.plan_mode_state.as_mut(),
                &mut self.assistant_message_stream_parsers,
                &item_id,
            )
            .await;
        }
        if let Some(state) = self.plan_mode_state.as_mut()
            && handle_assistant_item_done_in_plan_mode(
                sess,
                turn_context,
                &item,
                state,
                previously_active_item.as_ref(),
                &mut self.last_agent_message,
            )
            .await
        {
            return Ok(None);
        }

        let mut ctx = HandleOutputCtx {
            sess: Arc::clone(sess),
            turn_context: Arc::clone(turn_context),
            tool_runtime: tool_runtime.clone(),
            cancellation_token: cancellation_token.child_token(),
        };
        let output_result =
            crate::stream_events_utils::handle_output_item_done(&mut ctx, item, previously_active_item)
                .instrument(handle_responses)
                .await?;
        if let Some(agent_message) = output_result.last_agent_message {
            self.last_agent_message = Some(agent_message);
        }
        self.needs_follow_up |= output_result.needs_follow_up;
        Ok(output_result.tool_future)
    }

    pub(super) async fn handle_output_item_added(
        &mut self,
        sess: &Session,
        turn_context: &TurnContext,
        item: ResponseItem,
    ) {
        let plan_mode = self.plan_mode_state.is_some();
        if let Some(turn_item) = crate::stream_events_utils::handle_non_tool_response_item(&item, plan_mode) {
            let mut turn_item = turn_item;
            let mut seeded_parsed: Option<ParsedAssistantTextDelta> = None;
            let mut seeded_item_id: Option<String> = None;
            if matches!(turn_item, TurnItem::AgentMessage(_))
                && let Some(raw_text) = crate::stream_events_utils::raw_assistant_output_text_from_item(&item)
            {
                let item_id = turn_item.id();
                let mut seeded = self
                    .assistant_message_stream_parsers
                    .seed_item_text(&item_id, &raw_text);
                if let TurnItem::AgentMessage(agent_message) = &mut turn_item {
                    agent_message.content =
                        vec![codex_protocol::items::AgentMessageContent::Text {
                            text: if plan_mode {
                                String::new()
                            } else {
                                std::mem::take(&mut seeded.visible_text)
                            },
                        }];
                }
                seeded_parsed = plan_mode.then_some(seeded);
                seeded_item_id = Some(item_id);
            }
            if let Some(state) = self.plan_mode_state.as_mut()
                && matches!(turn_item, TurnItem::AgentMessage(_))
            {
                let item_id = turn_item.id();
                state
                    .pending_agent_message_items
                    .insert(item_id, turn_item.clone());
            } else {
                sess.emit_turn_item_started(turn_context, &turn_item).await;
            }
            if let (Some(state), Some(item_id), Some(parsed)) = (
                self.plan_mode_state.as_mut(),
                seeded_item_id.as_deref(),
                seeded_parsed,
            ) {
                emit_streamed_assistant_text_delta(sess, turn_context, Some(state), item_id, parsed)
                    .await;
            }
            match turn_item {
                TurnItem::AgentMessage(_) => self.active_agent_message_item = Some(turn_item),
                TurnItem::Reasoning(_) => self.active_reasoning_item = Some(turn_item),
                TurnItem::WebSearch(_) => self.active_web_search_item = Some(turn_item),
                TurnItem::UserMessage(_)
                | TurnItem::Plan(_)
                | TurnItem::ContextCompaction(_)
                | TurnItem::ImageGeneration(_) => {}
            }
        }
    }

    pub(super) async fn handle_output_text_delta(
        &mut self,
        sess: &Session,
        turn_context: &TurnContext,
        delta: String,
    ) {
        if let Some(active) = self.active_agent_message_item.as_ref() {
            let item_id = active.id();
            if matches!(active, TurnItem::AgentMessage(_)) {
                let parsed = self
                    .assistant_message_stream_parsers
                    .parse_delta(&item_id, &delta);
                emit_streamed_assistant_text_delta(
                    sess,
                    turn_context,
                    self.plan_mode_state.as_mut(),
                    &item_id,
                    parsed,
                )
                .await;
            } else {
                let event = AgentMessageContentDeltaEvent {
                    thread_id: sess.conversation_id.to_string(),
                    turn_id: turn_context.sub_id.clone(),
                    item_id,
                    delta,
                };
                sess.send_event(turn_context, EventMsg::AgentMessageContentDelta(event))
                    .await;
            }
        } else {
            error_or_panic("OutputTextDelta without active item".to_string());
        }
    }

    pub(super) async fn handle_reasoning_summary_delta(
        &self,
        sess: &Session,
        turn_context: &TurnContext,
        delta: String,
        summary_index: i64,
    ) {
        if let Some(active) = self.active_reasoning_item.as_ref() {
            let event = ReasoningContentDeltaEvent {
                thread_id: sess.conversation_id.to_string(),
                turn_id: turn_context.sub_id.clone(),
                item_id: active.id(),
                delta,
                summary_index,
            };
            sess.send_event(turn_context, EventMsg::ReasoningContentDelta(event))
                .await;
        } else {
            error_or_panic("ReasoningSummaryDelta without active item".to_string());
        }
    }

    pub(super) async fn handle_reasoning_summary_part_added(
        &self,
        sess: &Session,
        turn_context: &TurnContext,
        summary_index: i64,
    ) {
        if let Some(active) = self.active_reasoning_item.as_ref() {
            let event = EventMsg::AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent {
                item_id: active.id(),
                summary_index,
            });
            sess.send_event(turn_context, event).await;
        } else {
            error_or_panic("ReasoningSummaryPartAdded without active item".to_string());
        }
    }

    pub(super) async fn handle_reasoning_content_delta(
        &self,
        sess: &Session,
        turn_context: &TurnContext,
        delta: String,
        content_index: i64,
    ) {
        if let Some(active) = self.active_reasoning_item.as_ref() {
            let event = ReasoningRawContentDeltaEvent {
                thread_id: sess.conversation_id.to_string(),
                turn_id: turn_context.sub_id.clone(),
                item_id: active.id(),
                delta,
                content_index,
            };
            sess.send_event(turn_context, EventMsg::ReasoningRawContentDelta(event))
                .await;
        } else {
            error_or_panic("ReasoningRawContentDelta without active item".to_string());
        }
    }

    pub(super) async fn flush_all(&mut self, sess: &Session, turn_context: &TurnContext) {
        flush_assistant_text_segments_all(
            sess,
            turn_context,
            self.plan_mode_state.as_mut(),
            &mut self.assistant_message_stream_parsers,
        )
        .await;
    }

    pub(super) async fn note_pending_follow_up(&mut self, sess: &Session) {
        self.needs_follow_up |= sess.has_pending_input().await;
    }

    pub(super) fn mark_turn_diff_ready(&mut self) {
        self.should_emit_turn_diff = true;
    }

    pub(super) fn should_emit_turn_diff(&self) -> bool {
        self.should_emit_turn_diff
    }

    pub(super) fn into_result(self) -> SamplingRequestResult {
        SamplingRequestResult {
            needs_follow_up: self.needs_follow_up,
            last_agent_message: self.last_agent_message,
        }
    }
}

#[derive(Debug)]
struct ProposedPlanItemState {
    item_id: String,
    started: bool,
    completed: bool,
}

#[derive(Debug)]
pub(super) struct PlanModeStreamState {
    pending_agent_message_items: HashMap<String, TurnItem>,
    started_agent_message_items: HashSet<String>,
    leading_whitespace_by_item: HashMap<String, String>,
    plan_item_state: ProposedPlanItemState,
}

impl PlanModeStreamState {
    fn new(turn_id: &str) -> Self {
        Self {
            pending_agent_message_items: HashMap::new(),
            started_agent_message_items: HashSet::new(),
            leading_whitespace_by_item: HashMap::new(),
            plan_item_state: ProposedPlanItemState::new(turn_id),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct AssistantMessageStreamParsers {
    plan_mode: bool,
    parsers_by_item: HashMap<String, AssistantTextStreamParser>,
}

pub(super) type ParsedAssistantTextDelta = AssistantTextChunk;

impl AssistantMessageStreamParsers {
    pub(super) fn new(plan_mode: bool) -> Self {
        Self {
            plan_mode,
            parsers_by_item: HashMap::new(),
        }
    }

    fn parser_mut(&mut self, item_id: &str) -> &mut AssistantTextStreamParser {
        let plan_mode = self.plan_mode;
        self.parsers_by_item
            .entry(item_id.to_string())
            .or_insert_with(|| AssistantTextStreamParser::new(plan_mode))
    }

    pub(super) fn seed_item_text(&mut self, item_id: &str, text: &str) -> ParsedAssistantTextDelta {
        if text.is_empty() {
            return ParsedAssistantTextDelta::default();
        }
        self.parser_mut(item_id).push_str(text)
    }

    pub(super) fn parse_delta(&mut self, item_id: &str, delta: &str) -> ParsedAssistantTextDelta {
        self.parser_mut(item_id).push_str(delta)
    }

    pub(super) fn finish_item(&mut self, item_id: &str) -> ParsedAssistantTextDelta {
        let Some(mut parser) = self.parsers_by_item.remove(item_id) else {
            return ParsedAssistantTextDelta::default();
        };
        parser.finish()
    }

    fn drain_finished(&mut self) -> Vec<(String, ParsedAssistantTextDelta)> {
        let parsers_by_item = std::mem::take(&mut self.parsers_by_item);
        parsers_by_item
            .into_iter()
            .map(|(item_id, mut parser)| (item_id, parser.finish()))
            .collect()
    }
}

impl ProposedPlanItemState {
    fn new(turn_id: &str) -> Self {
        Self {
            item_id: format!("{turn_id}-plan"),
            started: false,
            completed: false,
        }
    }

    async fn start(&mut self, sess: &Session, turn_context: &TurnContext) {
        if self.started || self.completed {
            return;
        }
        self.started = true;
        let item = TurnItem::Plan(PlanItem {
            id: self.item_id.clone(),
            text: String::new(),
        });
        sess.emit_turn_item_started(turn_context, &item).await;
    }

    async fn push_delta(&mut self, sess: &Session, turn_context: &TurnContext, delta: &str) {
        if self.completed || delta.is_empty() {
            return;
        }
        let event = PlanDeltaEvent {
            thread_id: sess.conversation_id.to_string(),
            turn_id: turn_context.sub_id.clone(),
            item_id: self.item_id.clone(),
            delta: delta.to_string(),
        };
        sess.send_event(turn_context, EventMsg::PlanDelta(event))
            .await;
    }

    async fn complete_with_text(
        &mut self,
        sess: &Session,
        turn_context: &TurnContext,
        text: String,
    ) {
        if self.completed || !self.started {
            return;
        }
        self.completed = true;
        let item = TurnItem::Plan(PlanItem {
            id: self.item_id.clone(),
            text,
        });
        sess.emit_turn_item_completed(turn_context, item).await;
    }
}

async fn maybe_emit_pending_agent_message_start(
    sess: &Session,
    turn_context: &TurnContext,
    state: &mut PlanModeStreamState,
    item_id: &str,
) {
    if state.started_agent_message_items.contains(item_id) {
        return;
    }
    if let Some(item) = state.pending_agent_message_items.remove(item_id) {
        sess.emit_turn_item_started(turn_context, &item).await;
        state
            .started_agent_message_items
            .insert(item_id.to_string());
    }
}

async fn handle_plan_segments(
    sess: &Session,
    turn_context: &TurnContext,
    state: &mut PlanModeStreamState,
    item_id: &str,
    segments: Vec<ProposedPlanSegment>,
) {
    for segment in segments {
        match segment {
            ProposedPlanSegment::Normal(delta) => {
                if delta.is_empty() {
                    continue;
                }
                let has_non_whitespace = delta.chars().any(|ch| !ch.is_whitespace());
                if !has_non_whitespace && !state.started_agent_message_items.contains(item_id) {
                    let entry = state
                        .leading_whitespace_by_item
                        .entry(item_id.to_string())
                        .or_default();
                    entry.push_str(&delta);
                    continue;
                }
                let delta = if !state.started_agent_message_items.contains(item_id) {
                    if let Some(prefix) = state.leading_whitespace_by_item.remove(item_id) {
                        format!("{prefix}{delta}")
                    } else {
                        delta
                    }
                } else {
                    delta
                };
                maybe_emit_pending_agent_message_start(sess, turn_context, state, item_id).await;

                let event = AgentMessageContentDeltaEvent {
                    thread_id: sess.conversation_id.to_string(),
                    turn_id: turn_context.sub_id.clone(),
                    item_id: item_id.to_string(),
                    delta,
                };
                sess.send_event(turn_context, EventMsg::AgentMessageContentDelta(event))
                    .await;
            }
            ProposedPlanSegment::ProposedPlanStart => {
                if !state.plan_item_state.completed {
                    state.plan_item_state.start(sess, turn_context).await;
                }
            }
            ProposedPlanSegment::ProposedPlanDelta(delta) => {
                if !state.plan_item_state.completed {
                    if !state.plan_item_state.started {
                        state.plan_item_state.start(sess, turn_context).await;
                    }
                    state
                        .plan_item_state
                        .push_delta(sess, turn_context, &delta)
                        .await;
                }
            }
            ProposedPlanSegment::ProposedPlanEnd => {}
        }
    }
}

async fn emit_streamed_assistant_text_delta(
    sess: &Session,
    turn_context: &TurnContext,
    plan_mode_state: Option<&mut PlanModeStreamState>,
    item_id: &str,
    parsed: ParsedAssistantTextDelta,
) {
    if parsed.is_empty() {
        return;
    }
    if !parsed.citations.is_empty() {
        let _citations = parsed.citations;
    }
    if let Some(state) = plan_mode_state {
        if !parsed.plan_segments.is_empty() {
            handle_plan_segments(sess, turn_context, state, item_id, parsed.plan_segments).await;
        }
        return;
    }
    if parsed.visible_text.is_empty() {
        return;
    }
    let event = AgentMessageContentDeltaEvent {
        thread_id: sess.conversation_id.to_string(),
        turn_id: turn_context.sub_id.clone(),
        item_id: item_id.to_string(),
        delta: parsed.visible_text,
    };
    sess.send_event(turn_context, EventMsg::AgentMessageContentDelta(event))
        .await;
}

async fn flush_assistant_text_segments_for_item(
    sess: &Session,
    turn_context: &TurnContext,
    plan_mode_state: Option<&mut PlanModeStreamState>,
    parsers: &mut AssistantMessageStreamParsers,
    item_id: &str,
) {
    let parsed = parsers.finish_item(item_id);
    emit_streamed_assistant_text_delta(sess, turn_context, plan_mode_state, item_id, parsed).await;
}

async fn flush_assistant_text_segments_all(
    sess: &Session,
    turn_context: &TurnContext,
    mut plan_mode_state: Option<&mut PlanModeStreamState>,
    parsers: &mut AssistantMessageStreamParsers,
) {
    for (item_id, parsed) in parsers.drain_finished() {
        emit_streamed_assistant_text_delta(
            sess,
            turn_context,
            plan_mode_state.as_deref_mut(),
            &item_id,
            parsed,
        )
        .await;
    }
}

async fn maybe_complete_plan_item_from_message(
    sess: &Session,
    turn_context: &TurnContext,
    state: &mut PlanModeStreamState,
    item: &ResponseItem,
) {
    if let ResponseItem::Message { role, content, .. } = item
        && role == "assistant"
    {
        let mut text = String::new();
        for entry in content {
            if let ContentItem::OutputText { text: chunk } = entry {
                text.push_str(chunk);
            }
        }
        if let Some(plan_text) = extract_proposed_plan_text(&text) {
            let (plan_text, _citations) = strip_citations(&plan_text);
            if !state.plan_item_state.started {
                state.plan_item_state.start(sess, turn_context).await;
            }
            state
                .plan_item_state
                .complete_with_text(sess, turn_context, plan_text)
                .await;
        }
    }
}

async fn emit_agent_message_in_plan_mode(
    sess: &Session,
    turn_context: &TurnContext,
    agent_message: codex_protocol::items::AgentMessageItem,
    state: &mut PlanModeStreamState,
) {
    let agent_message_id = agent_message.id.clone();
    let text = super::agent_message_text(&agent_message);
    if text.trim().is_empty() {
        state.pending_agent_message_items.remove(&agent_message_id);
        state.started_agent_message_items.remove(&agent_message_id);
        return;
    }

    maybe_emit_pending_agent_message_start(sess, turn_context, state, &agent_message_id).await;

    if !state.started_agent_message_items.contains(&agent_message_id) {
        let start_item = state
            .pending_agent_message_items
            .remove(&agent_message_id)
            .unwrap_or_else(|| {
                TurnItem::AgentMessage(codex_protocol::items::AgentMessageItem {
                    id: agent_message_id.clone(),
                    content: Vec::new(),
                    phase: None,
                })
            });
        sess.emit_turn_item_started(turn_context, &start_item).await;
        state
            .started_agent_message_items
            .insert(agent_message_id.clone());
    }

    sess.emit_turn_item_completed(turn_context, TurnItem::AgentMessage(agent_message))
        .await;
    state.started_agent_message_items.remove(&agent_message_id);
}

async fn emit_turn_item_in_plan_mode(
    sess: &Session,
    turn_context: &TurnContext,
    turn_item: TurnItem,
    previously_active_item: Option<&TurnItem>,
    state: &mut PlanModeStreamState,
) {
    match turn_item {
        TurnItem::AgentMessage(agent_message) => {
            emit_agent_message_in_plan_mode(sess, turn_context, agent_message, state).await;
        }
        _ => {
            if previously_active_item.is_none() {
                sess.emit_turn_item_started(turn_context, &turn_item).await;
            }
            sess.emit_turn_item_completed(turn_context, turn_item).await;
        }
    }
}

async fn handle_assistant_item_done_in_plan_mode(
    sess: &Session,
    turn_context: &TurnContext,
    item: &ResponseItem,
    state: &mut PlanModeStreamState,
    previously_active_item: Option<&TurnItem>,
    last_agent_message: &mut Option<String>,
) -> bool {
    if let ResponseItem::Message { role, .. } = item
        && role == "assistant"
    {
        maybe_complete_plan_item_from_message(sess, turn_context, state, item).await;

        if let Some(turn_item) = crate::stream_events_utils::handle_non_tool_response_item(item, true) {
            emit_turn_item_in_plan_mode(
                sess,
                turn_context,
                turn_item,
                previously_active_item,
                state,
            )
            .await;
        }

        crate::stream_events_utils::record_completed_response_item(sess, turn_context, item).await;
        if let Some(agent_message) =
            crate::stream_events_utils::last_assistant_message_from_item(item, true)
        {
            *last_agent_message = Some(agent_message);
        }
        return true;
    }
    false
}

pub(super) async fn drain_in_flight(
    in_flight: &mut FuturesOrdered<BoxFuture<'static, CodexResult<ResponseInputItem>>>,
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> CodexResult<()> {
    while let Some(res) = in_flight.next().await {
        match res {
            Ok(response_input) => {
                sess.record_conversation_items(&turn_context, &[response_input.into()])
                    .await;
            }
            Err(err) => {
                error_or_panic(format!("in-flight tool future failed during drain: {err}"));
            }
        }
    }
    Ok(())
}
