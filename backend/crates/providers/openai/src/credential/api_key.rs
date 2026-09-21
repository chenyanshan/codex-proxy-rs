//! API Key 账号的凭据合同；地址、传输策略与展示能力覆盖随凭据 revision 一起更新。

use std::collections::BTreeMap;
use std::fmt;

use gateway_core::routing::UpstreamModelId;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub const CODEX_AUTHENTICATION_KIND_API_KEY: &str = "api_key";

/// 单个模型的展示能力覆盖。
///
/// 上游 `/models` 只保证返回模型 ID，不保证提供模态字段，因此图片输入能力需要在账号上显式声明。
/// 覆盖只作用于该账号实际发现且模型策略允许的模型，不会凭空增加模型。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyModelPresentationOverride {
    #[serde(default)]
    pub image_input: bool,
    #[serde(default)]
    pub image_detail_original: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyTransport {
    #[default]
    Http,
    PreferWebsocket,
}

/// 可在管理端展示的上游设置，不包含密钥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyConfiguration {
    pub base_url: String,
    #[serde(default)]
    pub transport: ApiKeyTransport,
}

impl ApiKeyConfiguration {
    pub(crate) fn validate(&self) -> bool {
        self.base_url.len() <= 2048
            && !self.base_url.chars().any(char::is_control)
            && crate::transport::valid_upstream_base_url(&self.base_url)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyCredentialData {
    pub schema_version: u32,
    pub installation_id: String,
    pub api_key: String,
    pub base_url: String,
    #[serde(default)]
    pub transport: ApiKeyTransport,
    /// 账号级模型展示能力覆盖；键为上游模型 ID。
    #[serde(default, rename = "modelPresentationOverrides")]
    pub model_presentation_overrides: BTreeMap<String, ApiKeyModelPresentationOverride>,
}

impl ApiKeyCredentialData {
    pub fn configuration(&self) -> ApiKeyConfiguration {
        ApiKeyConfiguration {
            base_url: self.base_url.clone(),
            transport: self.transport,
        }
    }

    pub(crate) fn validate(&self) -> bool {
        self.schema_version == 1
            && self.configuration().validate()
            && !self.api_key.is_empty()
            && self.api_key.len() <= 16 * 1024
            && self.api_key.bytes().all(|byte| byte.is_ascii_graphic())
            && self
                .model_presentation_overrides
                .keys()
                .all(|id| UpstreamModelId::new(id.as_str()).is_ok())
    }
}

impl fmt::Debug for ApiKeyCredentialData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiKeyCredentialData")
            .field("schema_version", &self.schema_version)
            .field("configuration", &self.configuration())
            .field("api_key", &"<redacted>")
            .finish()
    }
}

pub struct ApiKeyAuthentication {
    pub configuration: ApiKeyConfiguration,
    pub secret: SecretString,
}

impl fmt::Debug for ApiKeyAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiKeyAuthentication")
            .field("configuration", &self.configuration)
            .field("secret", &"<redacted>")
            .finish()
    }
}
