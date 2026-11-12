//! Out, the cloud's half: an agent's token traded for that cloud's own credential naming the
//! agent. The shapes follow adk-rust's `agent-federation` (branch `feat/agent-identity-federation`)
//! and the live validation in `specs/saas/identity-validation.md`; this crate does not depend on
//! that branch, since the workspace is pinned at `3e797c38`.
//!
//! | Cloud | Audience of the agent's token | Exchange |
//! |---|---|---|
//! | AWS IAM | [`AWS_STS_AUDIENCE`] | `sts:AssumeRoleWithWebIdentity` (unsigned): temporary credentials of a role |
//! | Google Cloud | [`GoogleWorkloadIdentity::token_audience`] (`https://iam.googleapis.com/…/providers/<p>`) | STS token exchange at `https://sts.googleapis.com/v1/token`, whose `audience` is the `//iam.googleapis.com/…` form |
//! | Microsoft Entra | [`ENTRA_ASSERTION_AUDIENCE`] | two steps: the blueprint (`fmi_path` = the agent identity), then the agent identity with the first token as its assertion |
//!
//! Amazon Bedrock AgentCore's `GetWorkloadAccessTokenForJWT` (validated live) is signed with AWS
//! Signature Version 4 and is not built here. Tokens and secrets never appear in `Debug`.

use chrono::{DateTime, Duration, TimeZone, Utc};
use saas_core::federation::AgentTokenIssuer;
use serde_json::Value;

use crate::FederationError;

/// The audience AWS IAM's `AssumeRoleWithWebIdentity` expects.
pub const AWS_STS_AUDIENCE: &str = "sts.amazonaws.com";
/// The audience Entra's federated credential expects.
pub const ENTRA_ASSERTION_AUDIENCE: &str = "api://AzureADTokenExchange";
/// How long the agent's own token lives when it is traded at once.
pub const ASSERTION_TTL: Duration = Duration::minutes(5);

const JWT_BEARER: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// A secret that never prints.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// The secret itself, for the one place that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<withheld>)")
    }
}

fn unavailable(cloud: &str, e: impl std::fmt::Display) -> FederationError {
    FederationError::Unavailable(format!("{cloud}: {e}"))
}

fn refused(cloud: &str, status: reqwest::StatusCode, body: &str) -> FederationError {
    // The cloud's error, not the token: providers echo neither the assertion nor a secret.
    let detail: String = body.chars().take(300).collect();
    FederationError::Refused(format!("{cloud} answered {status}: {detail}"))
}

async fn assertion(
    issuer: &dyn AgentTokenIssuer,
    agent_did: &str,
    audience: &str,
) -> Result<String, FederationError> {
    Ok(issuer
        .issue(agent_did, audience, ASSERTION_TTL)
        .await
        .map_err(|e| FederationError::Key(e.to_string()))?
        .token)
}

/// An OAuth token endpoint's answer: `access_token` and `expires_in`.
async fn oauth(
    cloud: &str,
    http: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<(Secret, DateTime<Utc>), FederationError> {
    let resp = http
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| unavailable(cloud, e))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| unavailable(cloud, e))?;
    if !status.is_success() {
        return Err(refused(cloud, status, &body));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| unavailable(cloud, e))?;
    let token = v["access_token"]
        .as_str()
        .ok_or_else(|| FederationError::Invalid(format!("{cloud}: no access_token")))?;
    let expires_in = v["expires_in"]
        .as_i64()
        .or_else(|| v["expires_in"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(3600);
    Ok((
        Secret(token.to_owned()),
        Utc::now() + Duration::seconds(expires_in),
    ))
}

/// AWS IAM: a role that trusts the site's issuer and the agent's DID as the subject.
#[derive(Debug, Clone)]
pub struct AwsWebIdentity {
    /// The role.
    pub role_arn: String,
    /// The STS endpoint, `https://sts.<region>.amazonaws.com/`.
    pub endpoint: String,
}

/// Temporary AWS credentials naming the agent.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    /// The access key id.
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: Secret,
    /// The session token.
    pub session_token: Secret,
    /// When they lapse.
    pub expiration: DateTime<Utc>,
    /// The subject AWS read from the token: the agent's DID.
    pub subject: String,
    /// The assumed role's user ARN.
    pub assumed_role_arn: String,
}

impl AwsWebIdentity {
    /// A role in `region`.
    pub fn new(role_arn: impl Into<String>, region: &str) -> Self {
        Self {
            role_arn: role_arn.into(),
            endpoint: format!("https://sts.{region}.amazonaws.com/"),
        }
    }

    /// `AssumeRoleWithWebIdentity` with the agent's token (audience [`AWS_STS_AUDIENCE`]).
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        issuer: &dyn AgentTokenIssuer,
        agent_did: &str,
        session: &str,
    ) -> Result<AwsCredentials, FederationError> {
        let token = assertion(issuer, agent_did, AWS_STS_AUDIENCE).await?;
        let form = [
            ("Action", "AssumeRoleWithWebIdentity"),
            ("Version", "2011-06-15"),
            ("RoleArn", self.role_arn.as_str()),
            ("RoleSessionName", session),
            ("WebIdentityToken", token.as_str()),
            ("DurationSeconds", "900"),
        ];
        let resp = http
            .post(&self.endpoint)
            .header("accept", "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|e| unavailable("aws sts", e))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| unavailable("aws sts", e))?;
        if !status.is_success() {
            return Err(refused("aws sts", status, &body));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| unavailable("aws sts", e))?;
        let r = &v["AssumeRoleWithWebIdentityResponse"]["AssumeRoleWithWebIdentityResult"];
        let c = &r["Credentials"];
        let text = |v: &Value, what: &str| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| FederationError::Invalid(format!("aws sts: no {what}")))
        };
        let expiration = c["Expiration"]
            .as_f64()
            .and_then(|s| Utc.timestamp_opt(s as i64, 0).single())
            .or_else(|| {
                c["Expiration"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Utc))
            })
            .ok_or_else(|| FederationError::Invalid("aws sts: no Expiration".into()))?;
        Ok(AwsCredentials {
            access_key_id: text(&c["AccessKeyId"], "AccessKeyId")?,
            secret_access_key: Secret(text(&c["SecretAccessKey"], "SecretAccessKey")?),
            session_token: Secret(text(&c["SessionToken"], "SessionToken")?),
            expiration,
            subject: text(
                &r["SubjectFromWebIdentityToken"],
                "SubjectFromWebIdentityToken",
            )?,
            assumed_role_arn: text(&r["AssumedRoleUser"]["Arn"], "AssumedRoleUser")?,
        })
    }
}

/// Google Cloud workload identity federation: a pool provider that trusts the site's issuer.
#[derive(Debug, Clone)]
pub struct GoogleWorkloadIdentity {
    audience: String,
    /// The STS endpoint, `https://sts.googleapis.com/v1/token`.
    pub endpoint: String,
}

/// A Google federated access token.
#[derive(Debug, Clone)]
pub struct GoogleAccessToken {
    /// The token.
    pub access_token: Secret,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
}

impl GoogleWorkloadIdentity {
    /// The default scope of the exchanged token.
    pub const DEFAULT_SCOPE: &'static str = "https://www.googleapis.com/auth/cloud-platform";

    /// The provider `provider` of pool `pool` in project number `project_number`.
    pub fn new(project_number: &str, pool: &str, provider: &str) -> Self {
        Self {
            audience: format!(
                "//iam.googleapis.com/projects/{project_number}/locations/global/workloadIdentityPools/{pool}/providers/{provider}"
            ),
            endpoint: "https://sts.googleapis.com/v1/token".into(),
        }
    }

    /// The audience the agent's token must carry: `https://iam.googleapis.com/…/providers/<p>`.
    pub fn token_audience(&self) -> String {
        format!("https:{}", self.audience)
    }

    /// The STS exchange with the agent's token.
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        issuer: &dyn AgentTokenIssuer,
        agent_did: &str,
    ) -> Result<GoogleAccessToken, FederationError> {
        let token = assertion(issuer, agent_did, &self.token_audience()).await?;
        let form = [
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            ),
            ("audience", self.audience.as_str()),
            ("scope", Self::DEFAULT_SCOPE),
            (
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            ),
            ("subject_token_type", "urn:ietf:params:oauth:token-type:jwt"),
            ("subject_token", token.as_str()),
        ];
        let (access_token, expires_at) = oauth("google sts", http, &self.endpoint, &form).await?;
        Ok(GoogleAccessToken {
            access_token,
            expires_at,
        })
    }
}

/// Microsoft Entra Agent ID: a blueprint whose federated credential names the site's issuer and
/// the agent's DID, and the agent identity made from it.
#[derive(Debug, Clone)]
pub struct EntraAgentIdentity {
    /// `https://login.microsoftonline.com`.
    pub authority: String,
    /// The tenant.
    pub tenant_id: String,
    /// The blueprint's app id.
    pub blueprint_app_id: String,
    /// The agent identity's id.
    pub agent_identity_id: String,
}

/// An access token of the agent identity.
#[derive(Debug, Clone)]
pub struct EntraAccessToken {
    /// The token.
    pub access_token: Secret,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
    /// The scope it was issued for, `<resource>/.default`.
    pub scope: String,
}

impl EntraAgentIdentity {
    /// The agent identity `agent_identity_id` of blueprint `blueprint_app_id` in `tenant_id`.
    pub fn new(
        tenant_id: impl Into<String>,
        blueprint_app_id: impl Into<String>,
        agent_identity_id: impl Into<String>,
    ) -> Self {
        Self {
            authority: "https://login.microsoftonline.com".into(),
            tenant_id: tenant_id.into(),
            blueprint_app_id: blueprint_app_id.into(),
            agent_identity_id: agent_identity_id.into(),
        }
    }

    /// The two exchanges: the blueprint with the agent's token (audience
    /// [`ENTRA_ASSERTION_AUDIENCE`]) and `fmi_path` the agent identity; then the agent identity
    /// with that token as its assertion, for `resource`.
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        issuer: &dyn AgentTokenIssuer,
        agent_did: &str,
        resource: &str,
    ) -> Result<EntraAccessToken, FederationError> {
        let endpoint = format!(
            "{}/{}/oauth2/v2.0/token",
            self.authority.trim_end_matches('/'),
            self.tenant_id
        );
        let token = assertion(issuer, agent_did, ENTRA_ASSERTION_AUDIENCE).await?;
        let blueprint = [
            ("client_id", self.blueprint_app_id.as_str()),
            ("scope", "api://AzureADTokenExchange/.default"),
            ("grant_type", "client_credentials"),
            ("fmi_path", self.agent_identity_id.as_str()),
            ("client_assertion_type", JWT_BEARER),
            ("client_assertion", token.as_str()),
        ];
        let (blueprint_token, _) = oauth("entra (blueprint)", http, &endpoint, &blueprint).await?;
        let scope = format!("{}/.default", resource.trim_end_matches('/'));
        let identity = [
            ("client_id", self.agent_identity_id.as_str()),
            ("scope", scope.as_str()),
            ("grant_type", "client_credentials"),
            ("client_assertion_type", JWT_BEARER),
            ("client_assertion", blueprint_token.expose()),
        ];
        let (access_token, expires_at) =
            oauth("entra (agent identity)", http, &endpoint, &identity).await?;
        Ok(EntraAccessToken {
            access_token,
            expires_at,
            scope,
        })
    }
}
