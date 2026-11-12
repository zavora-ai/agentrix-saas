# Agent identity: live validation against AWS, Google Cloud and Microsoft Entra

This is the record behind spec v3's "cloud compatibility" claim. It was first run by hand from one
Mac with a small test issuer, and then again through adk-rust's own code (see the last section). Account, project and
tenant ids are kept out of this file on purpose.

**How it works.** `scripts/cloud-identity/agentid.py` makes an RS256 key (kept local, mode 0600),
a discovery document and a key set. The two public files sit in a storage bucket, which is the
issuer. The script then mints ten-minute tokens whose subject is the agent's DID. Each cloud is told
once to trust that issuer and that subject, and from then on needs no secret from us.

| Cloud | What accepted the Cashier's token | Refused, as it should |
|---|---|---|
| AWS IAM | `AssumeRoleWithWebIdentity` returned credentials; AWS reported the DID as the subject | Another agent's subject; a tampered signature |
| Amazon Bedrock AgentCore Identity | `GetWorkloadAccessTokenForJWT` returned a workload token for the agent's workload identity | A tampered signature |
| Google Cloud | The Security Token Service exchanged it for a Google access token | A wrong audience; a tampered signature |
| Microsoft Entra Agent ID | A real blueprint accepted it as a federated credential, and Entra issued a token for the agent identity | Another agent's subject |

Issuer: test issuer in a tagged S3 bucket (two public JSON files). Subject: did:web:kilele-bakery-sample.paypal.agentrix.website:agents:cashier. RS256.

## AWS (us-east-1), 21:47–21:49 EAT
- IAM OIDC provider + role with no permissions. AssumeRoleWithWebIdentity with the Cashier's token: ACCEPTED; AWS reports SubjectFromWebIdentityToken = the DID. Another agent's subject: REFUSED (AccessDenied). Tampered signature: REFUSED (InvalidIdentityToken).
- AgentCore Identity: GetWorkloadAccessToken for the workload identity: opaque token. GetWorkloadAccessTokenForJWT with a token from our issuer: ACCEPTED (AgentCore fetched our keys from the issuer; no authorizer had to be configured). Tampered: REFUSED ("Invalid Signature for bearerToken").

## Google Cloud (project zavora-ai), about 21:50 EAT
- Workload identity pool + OIDC provider pointing at the issuer (Google fetched the discovery document and keys itself; nothing uploaded). STS token exchange with the Cashier's token: ACCEPTED, federated access token returned. A call it has no permission for answers 403 PERMISSION_DENIED, not 401, so Google treats it as an authenticated principal. Wrong audience: REFUSED. Tampered signature: REFUSED.
- Audience Google expects in the token: https://iam.googleapis.com/projects/<num>/locations/global/workloadIdentityPools/<pool>/providers/<provider>. google.subject is mapped from sub (limit 127 characters; the DID is 68).

## Microsoft Entra (James's tenant), about 21:51 EAT
- Real Agent ID objects: an agent identity blueprint, its principal, a federated identity credential naming our issuer and the DID, and an agent identity created from the blueprint. All created with the signed-in Global Administrator through Graph v1.0/beta; the Azure CLI token was enough.
- Step 1: blueprint token request with client_assertion = the Cashier's token (aud api://AzureADTokenExchange) and fmi_path = the agent identity: ACCEPTED.
- Step 2: agent identity token request with client_assertion = step 1's token: ACCEPTED. The token's appid/sub/oid are the agent identity; xms_par_app_azp is the blueprint; xms_act_fct "3 11 9" and xms_sub_fct "11 3 9" mark an agent identity; idtyp is "app".
- Another agent's subject: REFUSED (AADSTS700213, no matching federated identity record).
- Entra accepted RS256 from an outside issuer and fetched the discovery document from the bucket.

## Inbound: a cloud's agent token checked by us (Entra), about 21:53 EAT
- The blueprint was given an identifier URI (api://<blueprint id>). The agent identity then asked for a token addressed to it. Result: an RS256 v2.0 token whose signature, issuer, audience and expiry verify against Entra's published keys with an ordinary JWT library.
- In that token: azp, sub and oid are the agent identity; aud is the blueprint's app id. The agent-marking claims seen on the Graph token (xms_act_fct, xms_sub_fct, xms_par_app_azp, idtyp) were NOT present. A verifier must not rely on them for a custom audience unless optional claims are configured.

## Inbound: a Google-hosted agent's token checked by us, 22:11 EAT
- A minimal agent (no model calls) was deployed on Google's Agent Runtime in us-central1 with agent identity. Google gave it an identity under the organisation's agent trust domain.
- Asked for an identity token addressed to our door, the agent returned an RS256 token. Its issuer is Google's Security Token Service path for the organisation's agent pool; that issuer publishes a discovery document and keys. Signature, issuer, audience and expiry verify with an ordinary JWT library. The same token offered to another audience is refused.
- The subject is the agent's SPIFFE id: spiffe://agents.global.org-<org>.system.id.goog/resources/aiplatform/projects/<num>/locations/us-central1/reasoningEngines/<id>. Claims: iss, sub, aud (an array), exp, iat, and an opaque "google" object.
- The token was requested unbound; a bound one would carry a cnf claim tying it to the agent's certificate.

## What this fixes for the implementation

- **Algorithm:** all three accepted RS256 from an outside issuer. None was tried with ES256 or EdDSA.
- **Discovery:** AWS, AgentCore and Entra fetched the issuer's keys themselves, so the issuer must be reachable over HTTPS. Google did too, and can also take an uploaded key set.
- **Audience per cloud:** `sts.amazonaws.com` (AWS IAM), the pool provider's resource name (Google), `api://AzureADTokenExchange` (Entra). AgentCore accepted the token without a configured audience.
- **Entra is two exchanges:** the blueprint first (with `fmi_path` naming the agent identity), then the agent identity with the first token as its assertion.
- **Reading a cloud's agent token:** for Entra, trust `azp`/`sub` as the agent and treat the agent-marking claims as optional. For Google, the subject is already a SPIFFE id and can be used as the agent's id unchanged.

## Not validated yet

- Google's certificate-bound tokens and mutual TLS. The token checked here was requested unbound.
- AWS-signed tokens for an AWS-hosted agent. AgentCore's workload token is opaque.
- An AWS-hosted agent's identity checked by us: it needs an account setting that only James can switch on.

## Resources and cleanup

Every object carried `project=agentrix-saas` and `cleanup=after-testing`. **All of them were removed
on Tue 10 Nov 2026 between 22:34 and 22:36 EAT**, once adk-rust's own live test had passed, and each
removal was checked: the storage buckets, the AWS identity provider and role, the AgentCore workload
identity, the Google pool, provider and agent runtime, and the Entra blueprint and agent identity.
Entra and Google keep soft-deleted copies for about 30 days. One setting stays on: Google's Security
Token Service API. The steps to set the objects up again are kept beside the test issuer's key on
the machine that ran this.

## The same, through adk-rust

adk-rust now does all of this itself: a new `adk-auth` feature, `agent-federation`, on the local
branch `feat/agent-identity-federation` (one commit on top of `origin/main` `fa5e22dbf`, not pushed,
no pull request yet). It holds the issuer, the four exchanges, and the reading of Entra and Google
agent tokens as an `AgentPrincipal`. It adds no dependency and changes no existing public type.

Its own live test ran against the same cloud objects on Tue 10 Nov at about 22:31 EAT:

```
issuer: published JWK set matches the signing key
aws sts: ACCEPTED; SubjectFromWebIdentityToken = did:web:kilele-bakery-sample.paypal.agentrix.website:agents:cashier; credentials valid for 59 minutes
aws sts: impostor subject REFUSED (403 AccessDenied)
agentcore: ACCEPTED; workload access token for workload agentrix-kilele-bakery-cashier
agentcore: tampered signature REFUSED (401 UnauthorizedException)
google sts: ACCEPTED; access token valid for 9 minutes
google sts: wrong audience REFUSED (400 invalid_grant)
google sts: CredentialBroker issued a token bound to https://storage.googleapis.com/
entra: ACCEPTED; agent identity token valid for 59 minutes
entra: impostor subject REFUSED (401 invalid_client, AADSTS700213)
entra inbound: VERIFIED; principal is urn:entra:<tenant>:agent:<agent identity>; attributes entra.agent_identity_id, entra.object_id, entra.tenant_id, entra.token_version
entra inbound: a verifier for another audience REFUSED the token
entra inbound: a Google access token REFUSED
google inbound: VERIFIED; principal id is the agent's SPIFFE id in the issuing pool; service aiplatform, resource type reasoningEngines
google inbound: a verifier for another audience REFUSED the token
test result: ok. 1 passed; 0 failed
```

Checks on that branch: `cargo fmt --check` clean, `cargo clippy -p adk-auth --features
agent-federation,sso --all-targets -D warnings` clean, 197 tests passed (35 of them new), the
changelog fragment valid.

**Open points before it goes upstream:**
- The id given to an Entra agent, `urn:entra:<tenant>:agent:<agent identity>`, is a choice to confirm.
- The AWS and AgentCore clients are not credential brokers: AWS credentials sign requests, and the
  AgentCore token is tied to no URL.
- The Entra broker path and Google's certificate-bound token were exercised only against mocks.
- Workspace-wide clippy and tests were not run, to stay inside the disk limit.
- One Signature Version 4 test vector was derived with botocore, not taken from AWS's published set.
