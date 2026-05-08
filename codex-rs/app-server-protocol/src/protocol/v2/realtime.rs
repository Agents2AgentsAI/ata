use super::*;

/// EXPERIMENTAL - thread realtime audio chunk.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeAudioChunk {
    pub data: String,
    pub sample_rate: u32,
    pub num_channels: u16,
    pub samples_per_channel: Option<u32>,
}

impl From<CoreRealtimeAudioFrame> for ThreadRealtimeAudioChunk {
    fn from(value: CoreRealtimeAudioFrame) -> Self {
        let CoreRealtimeAudioFrame {
            data,
            sample_rate,
            num_channels,
            samples_per_channel,
        } = value;
        Self {
            data,
            sample_rate,
            num_channels,
            samples_per_channel,
        }
    }
}

impl From<ThreadRealtimeAudioChunk> for CoreRealtimeAudioFrame {
    fn from(value: ThreadRealtimeAudioChunk) -> Self {
        let ThreadRealtimeAudioChunk {
            data,
            sample_rate,
            num_channels,
            samples_per_channel,
        } = value;
        Self {
            data,
            sample_rate,
            num_channels,
            samples_per_channel,
        }
    }
}

/// EXPERIMENTAL - start a thread-scoped realtime session.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeStartParams {
    pub thread_id: String,
    pub prompt: String,
    #[ts(optional = nullable)]
    pub session_id: Option<String>,
}

/// EXPERIMENTAL - response for starting thread realtime.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeStartResponse {}

/// EXPERIMENTAL - append audio input to thread realtime.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeAppendAudioParams {
    pub thread_id: String,
    pub audio: ThreadRealtimeAudioChunk,
}

/// EXPERIMENTAL - response for appending realtime audio input.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeAppendAudioResponse {}

/// EXPERIMENTAL - append text input to thread realtime.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeAppendTextParams {
    pub thread_id: String,
    pub text: String,
}

/// EXPERIMENTAL - response for appending realtime text input.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeAppendTextResponse {}

/// EXPERIMENTAL - stop thread realtime.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeStopParams {
    pub thread_id: String,
}

/// EXPERIMENTAL - response for stopping thread realtime.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeStopResponse {}

/// EXPERIMENTAL - emitted when thread realtime startup is accepted.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeStartedNotification {
    pub thread_id: String,
    pub session_id: Option<String>,
}

/// EXPERIMENTAL - raw non-audio thread realtime item emitted by the backend.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeItemAddedNotification {
    pub thread_id: String,
    pub item: JsonValue,
}

/// EXPERIMENTAL - streamed output audio emitted by thread realtime.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeOutputAudioDeltaNotification {
    pub thread_id: String,
    pub audio: ThreadRealtimeAudioChunk,
}

/// EXPERIMENTAL - emitted when thread realtime encounters an error.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeErrorNotification {
    pub thread_id: String,
    pub message: String,
}

/// EXPERIMENTAL - emitted when thread realtime transport closes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadRealtimeClosedNotification {
    pub thread_id: String,
    pub reason: Option<String>,
}
