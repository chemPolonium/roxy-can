//! ScriptNode methods: outbox.

use super::*;

impl ScriptNode {
    /// Hands the bus everything `emit_value` queued since the last call.
    pub fn take_emitted(&mut self) -> Vec<(String, f64)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.emitted))
            .unwrap_or_default()
    }

    /// Hands the bus everything `sys_set` queued since the last call.
    pub fn take_sys_sets(&mut self) -> Vec<(String, f64)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.pending_sys))
            .unwrap_or_default()
    }

    /// Hands the bus every `fr_send` queued since the last call: one FlexRay
    /// slot address plus the payload as the script wrote it.
    pub fn take_fr_out(&mut self) -> Vec<(u8, u16, Vec<u8>)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.fr_out))
            .unwrap_or_default()
    }

    /// Hands the bus every `set_fr_sig` queued since the last call.
    pub fn take_fr_sig_sets(&mut self) -> Vec<(u8, u16, String, f64)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.fr_sig))
            .unwrap_or_default()
    }

    /// Hands the bus every `send_frame` instance queued since the last call.
    pub fn take_frames(&mut self) -> Vec<crate::script::FrameInstance> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.frames))
            .unwrap_or_default()
    }

    pub fn take_someip(&mut self) -> Vec<crate::script::SomeIpSend> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.someip))
            .unwrap_or_default()
    }

    pub fn take_someip_tcp(&mut self) -> Vec<crate::script::SomeIpTcpSend> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.someip_tcp))
            .unwrap_or_default()
    }

    pub fn take_someip_offer(&mut self) -> Vec<crate::script::SomeIpOffer> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.someip_offer))
            .unwrap_or_default()
    }

    pub fn take_someip_reply(&mut self) -> Vec<crate::script::SomeIpReply> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.someip_reply))
            .unwrap_or_default()
    }

    pub fn take_nm_timing(&mut self) -> Vec<crate::script::NmTiming> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.nm_timing))
            .unwrap_or_default()
    }

    pub(super) fn fail(&mut self, msg: &str) {
        self.errored = true;
        self.issue = runtime_issue(msg);
        self.push_log(format!("[error] {msg}"));
    }

    /// Bus-side advisory into the node's log ring (an undefined system
    /// variable write, today): visible without failing the node.
    pub fn note(&mut self, line: String) {
        self.push_log(line);
    }
}
