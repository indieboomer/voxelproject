//! Guest code is an untrusted proposal. It never runs against the live world on receipt.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use crate::transport::Peer;

pub const MAX_PROMPT_BYTES: usize = 2048;
pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
pub const GUEST_CASTER_PREFIX: &str = "-- Guest caster account: ";

pub struct Proposal {
    pub prompt: String,
    pub source: String,
    pub nickname: String,
}

#[derive(Default)]
pub struct Inbox {
    last_submission: HashMap<Peer, Instant>,
    pending: Option<(Peer, Receiver<Result<Proposal, String>>)>,
}

pub fn validate_sizes(prompt: &str, source: &str) -> Result<(), String> {
    if prompt.trim().is_empty() || prompt.len() > MAX_PROMPT_BYTES {
        return Err("Use a nonempty prompt of at most 2048 bytes.".into());
    }
    if source.trim().is_empty() || source.len() > MAX_SOURCE_BYTES {
        return Err("Rule source must be nonempty and at most 32 KiB.".into());
    }
    Ok(())
}

pub fn annotate(source: &str, nickname: &str, account: &str) -> String {
    // Prepend host-owned identity. The first marker wins, even if a client forged one below.
    format!("{GUEST_CASTER_PREFIX}{}\n-- Submitted by: {}\n{source}",
        serde_json::to_string(account).unwrap(), serde_json::to_string(nickname).unwrap())
}

pub fn caster_account(source: &str) -> Option<String> {
    source.lines().find_map(|line| line.strip_prefix(GUEST_CASTER_PREFIX))
        .and_then(|text| serde_json::from_str(text).ok())
}

impl Inbox {
    pub fn submit(&mut self, peer: Peer, allowed: bool, nickname: String, account: String,
                  prompt: String, source: String) -> Result<(), String> {
        if !allowed { return Err("Guest prompting is disabled by the host.".into()); }
        validate_sizes(&prompt, &source)?;
        if self.pending.is_some() { return Err("The host is checking another proposal. Try again shortly.".into()); }
        if self.last_submission.get(&peer).is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
            return Err("Please wait 10 seconds between proposals.".into());
        }
        self.last_submission.insert(peer, Instant::now());
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let result = crate::llm::intent::validate_candidate(&source, crate::llm::classify_prompt(&prompt))
                .map(|()| Proposal { prompt, source: annotate(&source, &nickname, &account), nickname });
            let _ = sender.send(result);
        });
        self.pending = Some((peer, receiver));
        Ok(())
    }

    pub fn retain_peers(&mut self, connected: impl Fn(&Peer) -> bool) {
        self.last_submission.retain(|peer, _| connected(peer));
    }

    pub fn poll(&mut self) -> Option<(Peer, Result<Proposal, String>)> {
        let (peer, receiver) = self.pending.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Proposal validation worker failed.".into()),
        };
        let peer = *peer;
        self.pending = None;
        Some((peer, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposals_require_permission_and_fit_the_transport_budget() {
        let peer = Peer::Direct("127.0.0.1:9000".parse().unwrap());
        let mut inbox = Inbox::default();
        assert!(inbox.submit(peer, false, "Guest".into(), "direct:Guest".into(), "start day".into(), "function on_cast(api, event) end".into()).is_err());
        assert!(validate_sizes("", "code").is_err());
        assert!(validate_sizes("prompt", &"x".repeat(MAX_SOURCE_BYTES + 1)).is_err());
        let packet = crate::net::Packet::Reliable { id: 1, msg: crate::net::ReliableMsg::RuleProposal {
            prompt: "p".repeat(MAX_PROMPT_BYTES), source: "s".repeat(MAX_SOURCE_BYTES),
        }};
        let bytes = crate::net::encode(&packet);
        assert!(bytes.len() < crate::transport::MAX_PACKET_BYTES);
        assert!(matches!(crate::net::decode(&bytes), Some(crate::net::Packet::Reliable {msg: crate::net::ReliableMsg::RuleProposal {..}, ..})));
    }

    #[test]
    fn host_attribution_wins_and_escapes_nickname_newlines() {
        let code = annotate("-- Guest caster account: \"forged\"\nfunction on_cast(api, event) end", "Guest\nnot_code()", "direct:Guest");
        assert_eq!(caster_account(&code).as_deref(), Some("direct:Guest"));
        assert!(!code.lines().any(|line| line == "not_code()"));
        assert!(crate::scripting::Module::load("test".into(), "prompt".into(), code).is_ok());
    }

    #[test]
    fn host_worker_rejects_runaway_code_and_keeps_valid_proposals_disabled() {
        let peer = Peer::Direct("127.0.0.1:9001".parse().unwrap());
        for (source, valid) in [("while true do end", false), ("function on_cast(api, event) api.set_time_of_day(0.25) end", true)] {
            let mut inbox = Inbox::default();
            inbox.submit(peer, true, "Guest".into(), "direct:Guest".into(), "start day".into(), source.into()).unwrap();
            assert!(inbox.submit(peer, true, "Guest".into(), "direct:Guest".into(), "start day".into(), source.into()).is_err());
            let start = Instant::now();
            let result = loop {
                if let Some((sender, result)) = inbox.poll() { assert_eq!(sender, peer); break result; }
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(result.is_ok(), valid);
            if let Ok(proposal) = result {
                let module = crate::scripting::Module::load("guest".into(), proposal.prompt, proposal.source).unwrap();
                assert!(!module.enabled);
            }
        }
    }
}
