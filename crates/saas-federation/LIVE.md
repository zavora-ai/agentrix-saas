# Live proofs with the clouds

**What has been proven:** all three clouds accepted an agent's RS256 token from an outside issuer on Tue 10 Nov 2026: AWS IAM, Amazon Bedrock AgentCore, Google Cloud and Microsoft Entra Agent ID. That was done first with a test issuer, then through adk-rust's own code. The record is `specs/saas/identity-validation.md`.

**The cloud objects have been removed.** The steps to set them up again are in `.agentrix-local-w0/cloud-identity/RECREATE.md`, on the machine that ran the validation.

This crate builds the same exchanges, offline-tested against mocks (`scripts/wcargo test -p saas-federation`). The live run waits for the deploy to serve the issuer on the judged host.

## The issuer

- **Hosts:** they come from the node's `platform_host` setting (`paypal.agentrix.website`), never hard-coded.
  - A site's issuer: `issuer_url(&site_host(slug, platform_host))`, for example `https://kilele-bakery-sample.paypal.agentrix.website`.
  - Agentrix's own: `issuer_url(platform_host)`.
- **What must be served:** the discovery document and key set must answer over HTTPS. AWS, AgentCore and Entra each fetched the keys themselves.
- **The key:** `SiteIssuer::for_host(host, key_dir)`. RSA 2048, mode 0600, `kid` = the RFC 7638 thumbprint.
- **The token:** `sub` is the agent's DID, for example `did:web:kilele-bakery-sample.paypal.agentrix.website:agents:cashier`, which is 68 characters (Google's limit is 127). RS256; all three clouds accepted it.

## Out: each cloud's exchange (`src/exchange.rs`)

| Cloud | Type | Audience of the agent's token | What comes back |
|---|---|---|---|
| AWS IAM | `AwsWebIdentity::new(role_arn, region)` | `sts.amazonaws.com` | `AssumeRoleWithWebIdentity` credentials; `subject` = the DID |
| Google Cloud | `GoogleWorkloadIdentity::new(project_number, pool, provider)` | `https://iam.googleapis.com/projects/<n>/locations/global/workloadIdentityPools/<pool>/providers/<provider>` | a federated access token |
| Microsoft Entra | `EntraAgentIdentity::new(tenant, blueprint_app_id, agent_identity_id)` | `api://AzureADTokenExchange` | the agent identity's access token |

**Google's two forms.** The STS request's own `audience` field is the `//iam.googleapis.com/…` form; only the token carries the `https:` form.

**Entra takes two exchanges:**
1. The blueprint, with `client_assertion` = the agent's token and `fmi_path` = the agent identity.
2. The agent identity, with `client_assertion` = the first token, for `<resource>/.default`.

**Not built here:**
- AgentCore's `GetWorkloadAccessTokenForJWT`. It was validated live, but it needs AWS Signature Version 4.

## In: their agents at our door (`Federation`, `AgentTokens`)

| Profile | Principal | From |
|---|---|---|
| `entra` | `urn:entra:<tenant>:agent:<azp>`, with attributes `entra.tenant_id`, `entra.agent_identity_id`, `entra.object_id`, `entra.token_version`; the blueprint is the class when present | Entra's tenant issuer. Validated live: the agent-marking claims are absent for a custom audience, so they are optional |
| `google` | the agent's SPIFFE id, unchanged, with `google.service`, `google.resource_type` and the other `google.*` attributes parsed from its path | Google's STS issuer for the organisation's agent pool. Validated live, with an unbound token |
| `agentcore` | `urn:aws:agentcore:<issuer>:<client_id or sub>` | the issuer the AgentCore agent is configured with. **Not validated:** it needs an account setting only James can switch on |
| `agentrix` | the agent's `did:web`, issued only by its own site | our sites |

## To run live, once the issuer is served

1. Recreate the objects (`RECREATE.md`), naming the deployed issuer URL and the agent's DID.
2. Call each exchange with `SiteIssuer::for_host(...)`.
   - Expect: AWS's subject equals the DID, Google returns an access token, and Entra returns the agent identity's token.
   - An impostor subject must be refused.
