//! Broker events → UI events (design §4.3a): the live receipt ticks, page progress and frames.
//!
//! One [`UiMapper`] per assessment or Move keeps cumulative totals across all its jobs, so a
//! `receipt-tick` is never a delta. Frames are already in control's CAS when they get here.

use std::collections::BTreeSet;

use agentrix_types::ui::{FrameEvent, FrameKind, PageEvent, PageStatus, TickEvent};
use agentrix_types::{JobEvent, JobId, UiEvent};
use chrono::{DateTime, Utc};

use super::fetch::parse_note;
use super::{BrokerEvent, FrameInfo};

/// Turns broker events into UI events.
#[derive(Debug, Clone, Default)]
pub struct UiMapper {
    job: Option<JobId>,
    hosts: BTreeSet<String>,
    requests: u64,
    bytes: u64,
    refusals: u64,
    /// URLs of the current fetch job, by index, with their page numbers.
    pages: Vec<(String, u32)>,
    /// Pages in all, for `page` events.
    of: u32,
    /// The URL the last note was about, for the next frame.
    last_url: Option<String>,
}

impl UiMapper {
    /// A mapper with nothing counted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts mapping a fetch job whose `urls` are pages `first..` of `of`. URLs that are
    /// not pages (robots.txt, a sitemap) pass `None` and get no `page` event.
    pub fn fetch_job(&mut self, urls: &[(String, Option<u32>)], of: u32) {
        self.pages = urls
            .iter()
            .map(|(u, n)| (u.clone(), n.unwrap_or(0)))
            .collect();
        self.of = of;
    }

    /// The totals so far.
    pub fn tick(&self, at: DateTime<Utc>) -> TickEvent {
        TickEvent {
            job: self.job.clone(),
            hosts: self.hosts.len() as u64,
            requests: self.requests,
            bytes: self.bytes,
            refusals: self.refusals,
            at,
        }
    }

    /// The UI events for one broker event.
    pub fn on(&mut self, ev: &BrokerEvent, at: DateTime<Utc>) -> Vec<UiEvent> {
        match ev {
            BrokerEvent::Admitted { job_id } => {
                self.job = Some(job_id.clone());
                vec![UiEvent::ReceiptTick(self.tick(at))]
            }
            BrokerEvent::Frame(info) => self.frame(info, at).into_iter().collect(),
            BrokerEvent::Job(job) => match job {
                JobEvent::Request {
                    host, bytes_down, ..
                } => {
                    self.hosts.insert(host.clone());
                    self.requests += 1;
                    self.bytes += bytes_down;
                    vec![UiEvent::ReceiptTick(self.tick(at))]
                }
                JobEvent::Refused(_) => {
                    self.refusals += 1;
                    vec![UiEvent::ReceiptTick(self.tick(at))]
                }
                JobEvent::Note { text } => {
                    let Some(note) = parse_note(text) else {
                        return Vec::new();
                    };
                    let Some((url, n)) = self.pages.get(note.i).cloned() else {
                        return Vec::new();
                    };
                    self.last_url = Some(url.clone());
                    if n == 0 {
                        return Vec::new();
                    }
                    let (status, reason) = match note.status {
                        Some(s) if s < 400 => (PageStatus::Read, None),
                        Some(s) => (PageStatus::Failed, Some(format!("HTTP {s}"))),
                        None => (PageStatus::Failed, Some("no answer".to_owned())),
                    };
                    vec![UiEvent::Page(PageEvent {
                        n,
                        of: self.of,
                        url,
                        status,
                        reason,
                        at,
                    })]
                }
                _ => Vec::new(),
            },
        }
    }

    /// A `frame` event for a stored frame: always a sealed screenshot, never "live" (N8).
    pub fn frame(&self, info: &FrameInfo, at: DateTime<Utc>) -> Option<UiEvent> {
        Some(UiEvent::Frame(FrameEvent::new(
            FrameKind::SealedScreenshot,
            self.last_url.clone().unwrap_or_default(),
            info.digest,
            info.w,
            info.h,
            at,
        )))
    }
}
