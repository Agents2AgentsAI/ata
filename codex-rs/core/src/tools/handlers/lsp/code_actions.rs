use super::*;

pub(super) fn filter_previewable_code_actions(
    actions: &[CodeActionOrCommand],
) -> (Vec<CodeAction>, Vec<CodeAction>) {
    let mut ready = Vec::new();
    let mut resolvable = Vec::new();
    for item in actions {
        if let CodeActionOrCommand::CodeAction(action) = item {
            if action.command.is_some() {
                continue;
            }
            if action.edit.is_some() {
                ready.push(action.clone());
            } else if action.data.is_some() {
                resolvable.push(action.clone());
            }
        }
    }
    (ready, resolvable)
}

pub(super) fn choose_code_action(
    actions: &[CodeAction],
    title: Option<&str>,
) -> Result<CodeAction, FunctionCallError> {
    if let Some(title) = title {
        if let Some(action) = actions.iter().find(|action| action.title == title) {
            return Ok(action.clone());
        }
        return Err(FunctionCallError::RespondToModel(format!(
            "no code action titled '{title}'. Available:\n- {}",
            actions
                .iter()
                .map(|action| action.title.as_str())
                .collect::<Vec<_>>()
                .join("\n- ")
        )));
    }

    if actions.len() == 1 {
        return Ok(actions[0].clone());
    }

    Err(FunctionCallError::RespondToModel(format!(
        "multiple previewable code actions found; pass `title` to select one:\n- {}",
        actions
            .iter()
            .map(|action| action.title.as_str())
            .collect::<Vec<_>>()
            .join("\n- ")
    )))
}
