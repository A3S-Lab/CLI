use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum SecretMutation {
    #[default]
    Unchanged,
    Clear,
    Inline {
        value: String,
    },
    Environment {
        variable: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HeaderInput {
    pub name: String,
    #[serde(default)]
    pub value: SecretMutation,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProviderInput {
    pub id: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub credential: SecretMutation,
    #[serde(default)]
    pub session_id_header: Option<String>,
    #[serde(default)]
    pub headers: Vec<HeaderInput>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ModalitiesInput {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CostInput {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LimitInput {
    #[serde(default)]
    pub context: Option<u32>,
    #[serde(default)]
    pub output: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ModelInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub credential: SecretMutation,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub session_id_header: Option<String>,
    #[serde(default)]
    pub headers: Vec<HeaderInput>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default = "default_true")]
    pub tool_call: bool,
    #[serde(default = "default_true")]
    pub temperature: bool,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub modalities: ModalitiesInput,
    #[serde(default)]
    pub cost: CostInput,
    #[serde(default)]
    pub limit: LimitInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum MutationDocument {
    UpsertProvider {
        provider: ProviderInput,
    },
    RemoveProvider {
        #[serde(rename = "providerId")]
        provider_id: String,
    },
    UpsertModel {
        #[serde(rename = "providerId")]
        provider_id: String,
        model: Box<ModelInput>,
    },
    RemoveModel {
        #[serde(rename = "providerId")]
        provider_id: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    UpdateRuntime {
        #[serde(default, rename = "thinkingBudget")]
        thinking_budget: Option<usize>,
        #[serde(default, rename = "llmApiTimeoutMs")]
        llm_api_timeout_ms: Option<u64>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProviderTestDocument {
    pub provider: ProviderInput,
}

fn default_true() -> bool {
    true
}
