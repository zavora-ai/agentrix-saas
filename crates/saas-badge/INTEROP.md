# Agent identity: how other systems map onto Agentrix badges

This is a mapping document only. The code that ships today has three `IdentityResolver`s (`src/resolve.rs`):
- `did:web`
- `did:key` (Ed25519)
- Web Bot Auth key directories

Every other system below would come in through the same trait, after Thursday.

**What an Agentrix agent is.** No agent identity standard is ratified. These are W3C DID identifiers with the did:web method. Agentrix hosts the sample sites, so one operator holds both a site's keys and the voucher's key.

| Agentrix | What it is |
|---|---|
| Identifier | `did:web:<site-host>:agents:<role>`, served by the site at `/agents/<role>/did.json` |
| Internal name | `urn:adk:agent:<company>/<role>` (adk-rust's `AgentId`), under `alsoKnownAs` |
| Key | Ed25519. The same key signs the agent's RFC 9421 requests at agent doors (Web Bot Auth, `kid` `agent:<key>`) |
| Sponsor | The site's owner (`Badge.sponsor`), whose passkey approves the PayPal link and the mandate |
| Badge | `/agents/<role>/badge.json`, signed by `did:web:<site-host>#owner` |
| Voucher | The operator, `did:web:<operator host>` (the configured `platform_host`, `paypal.agentrix.website` for the judged build), signs that it hosts the site and checked its PayPal link |
| Delegation | The owner's AP2-style intent mandate (signed once). Each payment is a cart mandate signed by the agent's key |
| On-behalf-of | adk-core's `ActingContext` on every governed call |
| Credentials | adk-auth's `CredentialBroker`: no secret ever enters a model |

## Microsoft Entra Agent ID

| Entra | Agentrix |
|---|---|
| Agent identity blueprint | The role's class (`AgentPrincipal.class`): Cashier, Receivables, Payables |
| Agent identity (service principal) | The agent's DID. The Entra object id would go under `alsoKnownAs` |
| Sponsor | `Badge.sponsor` (the owner) |
| Acting on behalf of a user | `ActingContext`, and the RFC 8693 token exchange per agent (saas-company) |
| Conditional access, revocation | Badge `status: frozen`, binding `status: revoked` |

A resolver would read the agent's Entra object through Microsoft Graph and check that it lists the DID. That needs a tenant and an app registration, so it isn't built.

## Amazon Bedrock AgentCore Identity

| AgentCore | Agentrix |
|---|---|
| Workload identity | `AgentId` (`urn:adk:agent:…`), and the DID |
| Token vault (OAuth tokens for outbound calls) | `CredentialBroker` |
| Inbound auth (JWT authorizer) | Web Bot Auth at the agent door, or OAuth on adk-server |

A resolver would map a workload identity name to the DID listed for it. That needs an AWS account, so it isn't built.

## Google: agent identity in the Agent Development Kit and Agent Engine

| Google | Agentrix |
|---|---|
| Agent principal | `AgentId`, and the DID |
| Credential vault and auth manager | `CredentialBroker` |
| Delegated OAuth | RFC 8693 token exchange (adk-auth `token-exchange`) |

Not built. It would need a Google Cloud project.

## OpenAgents Agent ID (`did:openagents:`)

A resolver for `did:openagents:` would ask OpenAgents' resolution endpoint and accept the agent only if the returned document lists the same Ed25519 key, or the `did:web` under `alsoKnownAs`. That needs James's account and API key, so it isn't built.

## Open Agent Identity (OAI-1)

From the same keys, publish:
- a `_oai-verify` DNS TXT record on the site's domain, naming the key's thumbprint
- `/.well-known/agent-identity.json`, listing each agent's DID and key

The record means something only if the zone is signed (DNSSEC). Not checked on Vultr DNS, so it isn't built.

## How a new system plugs in

```rust
#[async_trait]
impl IdentityResolver for EntraResolver {
    fn handles(&self, id: &str) -> bool { id.starts_with("entra:") }
    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError> { /* Graph lookup → keys, alsoKnownAs */ }
}
let resolvers = Resolvers::standard(http, directories).with(Arc::new(EntraResolver::new(/* … */)));
```

The verifier (`src/verify.rs`) only asks a resolver for keys by identifier. Any system that can name an Ed25519 key for an identifier can check an Agentrix badge, a voucher or a binding.
