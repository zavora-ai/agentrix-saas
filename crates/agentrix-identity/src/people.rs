//! People: the console app (contract C9) where an owner sees who helps run the site and
//! invites, changes or removes them (01b §4.2, 07 §5.6). W5's shell signs the owner in, checks
//! they own the site, and wraps this page at `/console/{site}/apps/people`.
//!
//! The page is server-rendered in the shell's classes. Inviting and changing a role need the
//! owner's passkey (or authenticator code) over that exact grant, so a small island,
//! `/team/people.js`, runs the step-up against [`crate::Team`]'s JSON routes. Removing someone
//! is a plain form post: it only ever lowers access.

use agentrix_shell::apps::{AppContext, AppPage, AppReply, AppSpec, ConsoleApp};
use agentrix_types::SiteId;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};

use crate::shell::escape;
use crate::team::{Role, Team};

/// The People app.
#[derive(Debug, Clone)]
pub struct PeopleApp {
    team: Team,
}

impl PeopleApp {
    /// Over the team service node built.
    pub fn new(team: Team) -> Self {
        Self { team }
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => "Owner",
        Role::Manager => "Manager",
        Role::Viewer => "Viewer",
        Role::Billing => "Billing",
    }
}

fn role_line(role: Role) -> &'static str {
    match role {
        Role::Owner => "Everything, including who helps run the site",
        Role::Manager => {
            "Approves the decisions their role covers. Can't add people or raise a WebAgent's authority"
        }
        Role::Viewer => "Sees everything. Changes nothing",
        Role::Billing => "Sees the site and pays invoices",
    }
}

const GRANTABLE: [Role; 3] = [Role::Manager, Role::Viewer, Role::Billing];

impl PeopleApp {
    async fn home(&self, ctx: &AppContext) -> anyhow::Result<String> {
        let site = SiteId::new(ctx.site_id.clone());
        let members = self
            .team
            .members_of(&site)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let site_name = escape(&ctx.site_name);
        let mut rows = format!(
            r#"<tr><td><b>{owner}</b><br><span class="small muted">You</span></td><td><span class="chip ok">Owner</span></td><td>{line}</td><td></td></tr>"#,
            owner = escape(&ctx.owner_name),
            line = role_line(Role::Owner),
        );
        for m in &members {
            let state = match &m.accepted {
                Some(when) => format!("Joined {}", escape(when)),
                None => format!(
                    "Invited {}. The invitation works for 7 days",
                    escape(&m.invited)
                ),
            };
            let options: String = GRANTABLE
                .iter()
                .map(|r| {
                    format!(
                        r#"<option value="{v}"{sel}>{n}</option>"#,
                        v = r.as_str(),
                        n = role_name(*r),
                        sel = if *r == m.role { " selected" } else { "" }
                    )
                })
                .collect();
            rows.push_str(&format!(
                r#"<tr><td><b>{email}</b><br><span class="small muted">{state}</span></td>
<td><form class="people-role" data-member="{id}"><label class="sr-only" for="role-{id}">Role for {email}</label><select id="role-{id}" name="role">{options}</select> <button class="btn btn-sm" type="submit">Change</button></form></td>
<td>{line}</td>
<td><form method="post" action="{base}/remove/{id}"><button class="btn btn-sm" type="submit">Remove</button></form></td></tr>"#,
                email = escape(&m.email),
                id = escape(&m.id),
                line = role_line(m.role),
                base = escape(&ctx.base),
            ));
        }
        let empty = if members.is_empty() {
            r#"<p class="small muted">Nobody else helps run this site yet. Invite someone below; they choose their own sign-in.</p>"#.to_owned()
        } else {
            String::new()
        };
        let sample = if ctx.sample {
            format!(
                r#"<p class="note" role="note"><span>{site_name} is a sample site. Invitations still need your passkey, and go only to the address you type.</span></p>"#
            )
        } else {
            String::new()
        };
        let role_options: String = GRANTABLE
            .iter()
            .map(|r| {
                format!(
                    r#"<option value="{}">{}: {}</option>"#,
                    r.as_str(),
                    role_name(*r),
                    role_line(*r)
                )
            })
            .collect();
        let explain: String = [Role::Owner, Role::Manager, Role::Viewer, Role::Billing]
            .iter()
            .map(|r| format!("<li><b>{}</b>: {}.</li>", role_name(*r), role_line(*r)))
            .collect();
        Ok(format!(
            r#"<div class="field-head"><div><h2>People</h2><div class="sub">Who can see and decide for {site_name}.</div></div></div>
{sample}
<div class="cards" id="people" data-site="{site_id}">
  <article class="fcard span2" aria-labelledby="people-h">
    <div class="fcard-head"><div class="who"><b id="people-h">On this site</b><span class="state-line">{count}</span></div></div>
    {empty}
    <table class="surface-table">
      <thead><tr><th scope="col">Person</th><th scope="col">Role</th><th scope="col">Can</th><th scope="col"><span class="sr-only">Remove</span></th></tr></thead>
      <tbody>{rows}</tbody>
    </table>
    <p class="small muted">Changing someone's role asks for your passkey. Removing someone takes effect at once.</p>
  </article>
  <article class="fcard" aria-labelledby="invite-h">
    <div class="fcard-head"><div class="who"><b id="invite-h">Invite someone</b><span class="state-line">Your passkey confirms each invitation</span></div></div>
    <form id="people-invite">
      <label class="o-field">Their email<input name="email" type="email" autocomplete="off" required></label>
      <label class="o-field">Role<select name="role">{role_options}</select></label>
      <label class="o-field" id="people-code" hidden>Code from your authenticator app<input name="totp" inputmode="numeric" autocomplete="one-time-code" maxlength="6"></label>
      <div class="verbs"><button class="btn btn-primary btn-sm" type="submit">Invite</button></div>
    </form>
    <p class="small muted">They get an email asking them to make an Agentrix account with that address. Accepting the invitation online isn't ready yet, so they stay listed as invited. Nobody can add people from your inbox alone.</p>
    <p class="note" id="people-status" role="status" hidden></p>
  </article>
  <article class="fcard" aria-labelledby="roles-h">
    <div class="fcard-head"><div class="who"><b id="roles-h">What each role can do</b></div></div>
    <ul>{explain}</ul>
    <p class="small muted">WebAgents never hold a role. Agencies get theirs from Trust, as a grant you can revoke.</p>
  </article>
</div>
<script src="/team/people.js" defer></script>"#,
            site_id = escape(&ctx.site_id),
            count = match members.len() {
                0 => "Just you".to_owned(),
                1 => "You and 1 more".to_owned(),
                n => format!("You and {n} more"),
            },
        ))
    }
}

#[async_trait]
impl ConsoleApp for PeopleApp {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "people".into(),
            name: "People".into(),
            title: "People: who can see and decide for this site".into(),
            icon: "people".into(),
            webagent: None,
            order: 40,
        }
    }

    async fn page(&self, ctx: &AppContext, path: &str, _query: &str) -> anyhow::Result<AppPage> {
        match path {
            "" => Ok(AppPage::Field {
                title: "People".into(),
                html: self.home(ctx).await?,
            }),
            _ => Ok(AppPage::NotFound),
        }
    }

    async fn post(
        &self,
        ctx: &AppContext,
        path: &str,
        headers: &HeaderMap,
        _body: Bytes,
    ) -> anyhow::Result<AppReply> {
        let Some(member) = path.strip_prefix("remove/") else {
            return Ok(AppReply::NotFound);
        };
        let site = SiteId::new(ctx.site_id.clone());
        match self.team.remove(&site, member, headers).await {
            Ok(_) => Ok(AppReply::Redirect(ctx.base.clone())),
            Err(e) => Ok(AppReply::Error(StatusCode::BAD_REQUEST, e.to_string())),
        }
    }
}
