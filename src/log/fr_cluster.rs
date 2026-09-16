//! FIBEX FlexRay cluster description parser.
//!
//! Extracts FlexRay cluster timing parameters from FIBEX 3.0 XML files
//! (Vector CANoe exports) and ARXML 4.x files (AUTOSAR ECU extracts),
//! filling a [`crate::hw::vector::flexray::XLfrClusterConfig`] for
//! `xlFrSetConfiguration`. Also extracts frame triggerings (slot
//! assignments) for frame identification.
//!
//! The parser is a flat line scanner — the FIBEX/ARXML structures are
//! regular enough that a full XML tree is unnecessary, and a flat scan
//! avoids pulling in a dedicated XML dependency.

/// One parsed FIBEX/ARXML parameter: a dotted path and a text value.
struct Param {
    path: String,
    value: String,
}

/// FlexRay cluster parameters extracted from a FIBEX/ARXML file, ready to
/// fill a `XLfrClusterConfig`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrClusterParams {
    pub g_cold_start_attempts: u32,
    pub g_listen_noise: u32,
    pub g_macro_per_cycle: u32,
    pub g_max_without_clock_correction_fatal: u32,
    pub g_max_without_clock_correction_passive: u32,
    pub g_network_management_vector_length: u32,
    pub g_number_of_minislots: u32,
    pub g_number_of_static_slots: u32,
    pub g_offset_correction_start: u32,
    pub g_payload_length_static: u32,
    pub g_sync_node_max: u32,
    pub gd_action_point_offset: u32,
    pub gd_dynamic_slot_idle_phase: u32,
    /// Macrotick duration in nanoseconds (FIBEX: μs × 1000).
    pub gd_macrotick_ns: u32,
    pub gd_minislot: u32,
    pub gd_mini_slot_action_point_offset: u32,
    pub gd_nit: u32,
    pub gd_static_slot: u32,
    pub gd_symbol_window: u32,
    pub gd_tss_transmitter: u32,
    pub gd_wakeup_symbol_rx_idle: u32,
    pub gd_wakeup_symbol_rx_low: u32,
    pub gd_wakeup_symbol_rx_window: u32,
    pub gd_wakeup_symbol_tx_idle: u32,
    pub gd_wakeup_symbol_tx_low: u32,
    pub p_allow_halt_due_to_clock: u32,
    pub p_allow_passive_to_active: u32,
    pub p_channels: u32,
    pub p_cluster_drift_damping: u32,
    pub p_decoding_correction: u32,
    pub p_delay_compensation_a: u32,
    pub p_delay_compensation_b: u32,
    pub p_extern_offset_correction: u32,
    pub p_extern_rate_correction: u32,
    pub p_key_slot_used_for_startup: u32,
    pub p_key_slot_used_for_sync: u32,
    pub p_latest_tx: u32,
    pub p_macro_initial_offset_a: u32,
    pub p_macro_initial_offset_b: u32,
    pub p_max_payload_length_dynamic: u32,
    pub p_micro_initial_offset_a: u32,
    pub p_micro_initial_offset_b: u32,
    pub p_micro_per_cycle: u32,
    pub p_micro_per_macro_nom: u32,
    pub p_offset_correction_out: u32,
    pub p_rate_correction_out: u32,
    pub p_samples_per_microtick: u32,
    pub p_single_slot_enabled: u32,
    pub p_wakeup_channel: u32,
    pub p_wakeup_pattern: u32,
    pub pd_accepted_startup_range: u32,
    pub pd_listen_timeout: u32,
    pub pd_max_drift: u32,
    /// Microtick duration in nanoseconds.
    pub pd_microtick_ns: u32,
    pub gd_cas_rx_low_max: u32,
    pub g_channels: u32,
    pub v_extern_offset_control: u32,
    pub v_extern_rate_control: u32,
    pub p_channels_mts: u32,
    pub frame_preset_data: u32,
    pub baudrate: u32,
}

/// One FlexRay frame assignment from the cluster description.
#[derive(Clone, Debug, PartialEq)]
pub struct FrFrameInfo {
    pub slot_id: u16,
    pub base_cycle: u8,
    pub cycle_repetition: u8,
    pub channel: u8,
}

/// Parses FIBEX XML (Vector CANoe export format) into cluster parameters.
/// Returns `None` if the file is not a FIBEX FlexRay description.
pub fn parse_fibex(text: &str) -> Option<(FrClusterParams, Vec<FrFrameInfo>)> {
    // Guard: the text must mention FlexRay AND have at least one FIBEX
    // FlexRay parameter tag — otherwise it's not a FIBEX FlexRay file.
    let has_fr_tag = text.contains("<flexray:") || text.contains("<fx:SPEED>");
    if (!text.contains("FlexRay") && !text.contains("flexray")) || !has_fr_tag {
        return None;
    }
    let mut p = FrClusterParams::default();

    // Simple flat parameter extraction: find `<flexray:TAG>value</flexray:TAG>`.
    let get = |tag: &str| -> Option<u32> {
        let open = format!("<flexray:{tag}>");
        let close = format!("</flexray:{tag}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&close)? + start;
        text[start..end].trim().parse().ok()
    };

    // The FIBEX `SPEED` is in bit/s (10000000 = 10 Mbit/s).
    if let Some(v) = get("SPEED").or_else(|| {
        // FIBEX uses fx:SPEED but the namespace prefix may vary.
        let open = "<fx:SPEED>";
        let start = text.find(open)? + open.len();
        let end = text[start..].find("</fx:SPEED>")? + start;
        text[start..end].trim().parse().ok()
    }) {
        p.baudrate = v;
    }

    macro_rules! fibex {
        ($field:expr, $tag:literal) => {
            if let Some(v) = get($tag) {
                $field = v;
            }
        };
    }
    fibex!(p.g_cold_start_attempts, "COLD-START-ATTEMPTS");
    fibex!(p.g_listen_noise, "LISTEN-NOISE");
    fibex!(p.g_macro_per_cycle, "MACRO-PER-CYCLE");
    fibex!(p.g_max_without_clock_correction_fatal, "MAX-WITHOUT-CLOCK-CORRECTION-FATAL");
    fibex!(p.g_max_without_clock_correction_passive, "MAX-WITHOUT-CLOCK-CORRECTION-PASSIVE");
    fibex!(p.g_network_management_vector_length, "NETWORK-MANAGEMENT-VECTOR-LENGTH");
    fibex!(p.g_number_of_minislots, "NUMBER-OF-MINISLOTS");
    fibex!(p.g_number_of_static_slots, "NUMBER-OF-STATIC-SLOTS");
    fibex!(p.g_offset_correction_start, "OFFSET-CORRECTION-START");
    fibex!(p.g_payload_length_static, "PAYLOAD-LENGTH-STATIC");
    fibex!(p.g_sync_node_max, "SYNC-NODE-MAX");
    fibex!(p.gd_action_point_offset, "ACTION-POINT-OFFSET");
    fibex!(p.gd_dynamic_slot_idle_phase, "DYNAMIC-SLOT-IDLE-PHASE");
    fibex!(p.gd_minislot, "MINISLOT");
    fibex!(p.gd_mini_slot_action_point_offset, "MINISLOT-ACTION-POINT-OFFSET");
    fibex!(p.gd_nit, "N-I-T");
    fibex!(p.gd_static_slot, "STATIC-SLOT");
    fibex!(p.gd_symbol_window, "SYMBOL-WINDOW");
    fibex!(p.gd_tss_transmitter, "T-S-S-TRANSMITTER");
    fibex!(p.gd_cas_rx_low_max, "CAS-RX-LOW-MAX");
    fibex!(p.gd_wakeup_symbol_rx_idle, "WAKE-UP-SYMBOL-RX-IDLE");
    fibex!(p.gd_wakeup_symbol_rx_low, "WAKE-UP-SYMBOL-RX-LOW");
    fibex!(p.gd_wakeup_symbol_rx_window, "WAKE-UP-SYMBOL-RX-WINDOW");
    fibex!(p.gd_wakeup_symbol_tx_idle, "WAKE-UP-SYMBOL-TX-IDLE");
    fibex!(p.gd_wakeup_symbol_tx_low, "WAKE-UP-SYMBOL-TX-LOW");
    fibex!(p.p_cluster_drift_damping, "CLUSTER-DRIFT-DAMPING");
    fibex!(p.p_decoding_correction, "DECODING-CORRECTION");
    fibex!(p.p_delay_compensation_a, "DELAY-COMPENSATION-A");
    fibex!(p.p_delay_compensation_b, "DELAY-COMPENSATION-B");
    fibex!(p.p_latest_tx, "LATEST-TX");
    fibex!(p.p_micro_per_cycle, "MICRO-PER-CYCLE");
    fibex!(p.p_offset_correction_out, "OFFSET-CORRECTION-OUT");
    fibex!(p.p_rate_correction_out, "RATE-CORRECTION-OUT");
    fibex!(p.p_samples_per_microtick, "SAMPLES-PER-MICROTICK");
    fibex!(p.pd_listen_timeout, "LISTEN-TIMEOUT");
    fibex!(p.pd_accepted_startup_range, "ACCEPTED-STARTUP-RANGE");
    fibex!(p.p_macro_initial_offset_a, "MACRO-INITIAL-OFFSET-A");
    fibex!(p.p_macro_initial_offset_b, "MACRO-INITIAL-OFFSET-B");
    fibex!(p.p_micro_initial_offset_a, "MICRO-INITIAL-OFFSET-A");
    fibex!(p.p_micro_initial_offset_b, "MICRO-INITIAL-OFFSET-B");

    // Float parameters that need unit conversion to integer nanoseconds.
    let get_f64 = |tag: &str| -> Option<f64> {
        let open = format!("<flexray:{tag}>");
        let close = format!("</flexray:{tag}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&close)? + start;
        text[start..end].trim().parse().ok()
    };

    // MACROTICK in μs → nanoseconds.
    if let Some(us) = get_f64("MACROTICK") {
        p.gd_macrotick_ns = (us * 1000.0).round() as u32;
    }
    // SAMPLE-CLOCK-PERIOD in μs → pdMicrotick = period × samples (in ns).
    let sample_clock_us = get_f64("SAMPLE-CLOCK-PERIOD").unwrap_or(0.0);
    if sample_clock_us > 0.0 && p.p_samples_per_microtick > 0 {
        p.pd_microtick_ns =
            (sample_clock_us * f64::from(p.p_samples_per_microtick) * 1000.0).round() as u32;
    }
    // BIT duration in μs → baudrate = 1e6 / bit_us.
    if let Some(us) = get_f64("BIT")
        && us > 0.0
    {
        p.baudrate = (1e6 / us) as u32;
    }
    // pMicroPerMacroNom = gdMacrotick / pdMicrotick.
    if p.gd_macrotick_ns > 0 && p.pd_microtick_ns > 0 {
        p.p_micro_per_macro_nom = p.gd_macrotick_ns / p.pd_microtick_ns;
    }

    // pMicroPerCycle = gMacroPerCycle × pMicroPerMacroNom if not set.
    if p.p_micro_per_cycle == 0 && p.p_micro_per_macro_nom > 0 {
        p.p_micro_per_cycle = p.g_macro_per_cycle * p.p_micro_per_macro_nom;
    }
    if p.p_micro_per_cycle == 0
        && let Some(v) = get("MICRO-PER-CYCLE")
    {
        p.p_micro_per_cycle = v;
    }

    // Boolean flags.
    let has_true = |tag: &str| -> bool {
        let open = format!("<flexray:{tag}>");
        match text.find(&open) {
            Some(start) => text[start + open.len()..].trim_start().starts_with("true"),
            None => false,
        }
    };    if has_true("ALLOW-HALT-DUE-TO-CLOCK") {
        p.p_allow_halt_due_to_clock = 1;
    }
    // Extract frame triggerings: slot/cycle assignments for frame IDs.
    let mut frames = Vec::new();
    let mut search = 0usize;
    while let Some(ft_start) = text[search..].find("<FLEXRAY-FRAME-TRIGGERING>") {
        let ft_off = search + ft_start;
        let ft_end = match text[ft_off..].find("</FLEXRAY-FRAME-TRIGGERING>") {
            Some(e) => ft_off + e,
            None => break,
        };
        let body = &text[ft_off..ft_end];
        let slot = body
            .find("<SLOT-ID>")
            .and_then(|s| {
                let vs = s + "<SLOT-ID>".len();
                let ve = body[vs..].find("</SLOT-ID>")? + vs;
                body[vs..ve].trim().parse().ok()
            })
            .unwrap_or(0);
        let cycle = body
            .find("<BASE-CYCLE>")
            .and_then(|s| {
                let vs = s + "<BASE-CYCLE>".len();
                let ve = body[vs..].find("</BASE-CYCLE>")? + vs;
                body[vs..ve].trim().parse().ok()
            })
            .unwrap_or(0);
        if slot > 0 {
            frames.push(FrFrameInfo {
                slot_id: slot,
                base_cycle: cycle,
                cycle_repetition: 1,
                channel: 0,
            });
        }
        search = ft_end;
    }
    p.p_channels = 3; // Channel A + B (the standard dual-channel setup)
    p.g_channels = 3;

    Some((p, frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FIBEX: &str = r#"
<?xml version="1.0" encoding="utf-8"?>
<fx:FIBEX xmlns:fx="http://www.asam.net/xml/fbx"
          xmlns:flexray="http://www.asam.net/xml/fbx/flexray">
  <fx:CLUSTERS>
    <fx:CLUSTER xsi:type="flexray:CLUSTER-TYPE">
      <ho:SHORT-NAME>A_FlexRay</ho:SHORT-NAME>
      <fx:SPEED>10000000</fx:SPEED>
      <flexray:COLD-START-ATTEMPTS>8</flexray:COLD-START-ATTEMPTS>
      <flexray:ACTION-POINT-OFFSET>2</flexray:ACTION-POINT-OFFSET>
      <flexray:MINISLOT>5</flexray:MINISLOT>
      <flexray:N-I-T>6</flexray:N-I-T>
      <flexray:SAMPLE-CLOCK-PERIOD>0.0125</flexray:SAMPLE-CLOCK-PERIOD>
      <flexray:STATIC-SLOT>43</flexray:STATIC-SLOT>
      <flexray:MACRO-PER-CYCLE>3636</flexray:MACRO-PER-CYCLE>
      <flexray:MACROTICK>1.375</flexray:MACROTICK>
      <flexray:NUMBER-OF-MINISLOTS>210</flexray:NUMBER-OF-MINISLOTS>
      <flexray:NUMBER-OF-STATIC-SLOTS>60</flexray:NUMBER-OF-STATIC-SLOTS>
      <flexray:OFFSET-CORRECTION-START>3632</flexray:OFFSET-CORRECTION-START>
      <flexray:PAYLOAD-LENGTH-STATIC>21</flexray:PAYLOAD-LENGTH-STATIC>
      <flexray:SYNC-NODE-MAX>15</flexray:SYNC-NODE-MAX>
      <flexray:CAS-RX-LOW-MAX>99</flexray:CAS-RX-LOW-MAX>
    </fx:CLUSTER>
  </fx:CLUSTERS>
</fx:FIBEX>
"#;

    #[test]
    fn fibex_parses_cluster_parameters() {
        let (p, _) = parse_fibex(SAMPLE_FIBEX).expect("FIBEX parses");
        assert_eq!(p.g_cold_start_attempts, 8);
        assert_eq!(p.g_macro_per_cycle, 3636);
        assert_eq!(p.g_number_of_static_slots, 60);
        assert_eq!(p.g_number_of_minislots, 210);
        assert_eq!(p.g_payload_length_static, 21);
        assert_eq!(p.gd_static_slot, 43);
        assert_eq!(p.gd_minislot, 5);
        assert_eq!(p.gd_macrotick_ns, 1375, "1.375 μs = 1375 ns");
        assert_eq!(p.baudrate, 10_000_000, "SPEED tag");
    }

    #[test]
    fn non_fibex_returns_none() {
        assert!(parse_fibex("<xml>no flexray here</xml>").is_none());
    }

    /// The real FIBEX file from the assets directory parses with the
    /// actual cluster parameters.
    #[test]
    fn real_fibex_file_parses() {
        let path = std::path::Path::new("assets/DemoFile_v3_FIBEX_3_0.xml");
        let Ok(text) = std::fs::read_to_string(path) else {
            println!("assets/DemoFile_v3_FIBEX_3_0.xml not present -- skipped");
            return;
        };
        let (p, _) = parse_fibex(&text).expect("the real FIBEX file parses");
        assert_eq!(p.baudrate, 10_000_000, "10 Mbit/s");
        assert_eq!(p.g_macro_per_cycle, 3636);
        assert_eq!(p.g_number_of_static_slots, 60);
        assert_eq!(p.g_number_of_minislots, 210);
        assert_eq!(p.g_payload_length_static, 21);
        assert_eq!(p.gd_static_slot, 43);
        assert_eq!(p.gd_minislot, 5);
        assert_eq!(p.gd_nit, 6);
        assert_eq!(p.gd_action_point_offset, 2);
        assert_eq!(p.g_cold_start_attempts, 8);
        assert!(p.p_micro_per_macro_nom > 0, "micro/macro ratio computed");
        assert_eq!(p.pd_microtick_ns, 25, "0.0125 μs × 2 samples = 25 ns");
    }
}
