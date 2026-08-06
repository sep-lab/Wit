//! Whitelist extraction: `Element` tree -> `wit_model::Model`. A field-for-
//! field port of `als_semantic_diff.py`'s `build_model()` — see that
//! module's docstring for why whitelisting, not blacklisting, is the rule.
//!
//! One behavioral fix from the Python original (M1 named bug fix 1): tempo
//! is read by searching the whole `LiveSet` subtree for the first
//! `Tempo/Manual`, not by requiring a specific host element tag. The Python
//! prototype only ever checks `MasterTrack`; Live 12.3 renamed that element
//! to `MainTrack`, so `build_model` silently returns `tempo=None` on every
//! current Live set (`tests/test_als_model.py`'s
//! `test_tempo_is_read_from_a_live_12_3_main_track`, xfail, documents the
//! bug). Searching generically for `Tempo/Manual` fixes this for both tags
//! and any future rename.

use crate::dom::{find_path, val, Element};
use std::collections::BTreeMap;
use wit_model::{Clip, Device, Fingerprint, Model, Track, TrackId, TrackKind};

fn num(s: Option<&str>) -> Option<f64> {
    s.and_then(|s| s.parse::<f64>().ok()).map(wit_model::round3)
}

/// A device's parameter fingerprint: BLAKE3 over every `<Manual Value="...">`
/// reachable under the device element, in document order. Whitelist, not
/// blacklist (AGENTS.md): only parameter *values* feed the hash, so id
/// churn elsewhere in the subtree (`LomId`, `AutomationTarget` ids, which
/// shift when an upstream element is inserted) cannot produce a false
/// "settings changed" positive. See `wit_model::Device`'s doc comment for
/// why this exists (M1 named bug fix 3, the knob-turn false negative).
fn device_fingerprint(device: &Element) -> Fingerprint {
    let mut hasher = blake3::Hasher::new();
    collect_manual_values(device, &mut hasher);
    Fingerprint(*hasher.finalize().as_bytes())
}

fn collect_manual_values(el: &Element, hasher: &mut blake3::Hasher) {
    for child in &el.children {
        if child.tag == "Manual" {
            if let Some(v) = child.attr("Value") {
                hasher.update(v.as_bytes());
                // A separator byte outside the value alphabet (XML attribute
                // values cannot contain a raw 0x1F) — without it,
                // ["1", "23"] and ["12", "3"] would hash identically.
                hasher.update(&[0x1f]);
            }
        }
        collect_manual_values(child, hasher);
    }
}

/// Extract the musically meaningful state of a Live set from its parsed XML
/// root (the `<Ableton>` element).
pub fn build_model(root: &Element) -> Model {
    let creator = root.attr("Creator").map(str::to_string);
    let mut model = Model {
        creator,
        tempo_bpm: None,
        tracks: BTreeMap::new(),
    };

    let Some(live_set) = root.find_child("LiveSet") else {
        return model;
    };

    if let Some(tempo_el) = live_set.find_descendant("Tempo") {
        model.tempo_bpm = num(tempo_el.find_child("Manual").and_then(|m| m.attr("Value")));
    }

    for tracks_el in live_set.find_all_children("Tracks") {
        for tr in &tracks_el.children {
            let Some(kind) = TrackKind::from_xml_tag(&tr.tag) else {
                continue; // e.g. PreHearTrack — not a whitelisted track kind
            };
            let Some(id) = tr.attr("Id") else { continue };

            let chain = tr.find_child("DeviceChain");
            let mixer = chain.and_then(|c| c.find_child("Mixer"));

            let name = find_path(tr, "Name/EffectiveName")
                .and_then(|e| e.attr("Value"))
                .unwrap_or("?")
                .to_string();

            let clips = extract_clips(chain);
            let devices = extract_devices(chain);

            let mut automation_lanes = Vec::new();
            tr.find_all_descendants("AutomationEnvelope", &mut automation_lanes);
            let mut notes = Vec::new();
            tr.find_all_descendants("MidiNoteEvent", &mut notes);

            model.tracks.insert(
                TrackId::from(id),
                Track {
                    id: TrackId::from(id),
                    name,
                    kind,
                    color: val(Some(tr), "Color").map(str::to_string),
                    volume: num(val(mixer, "Volume/Manual")),
                    pan: num(val(mixer, "Pan/Manual")),
                    speaker: val(mixer, "Speaker/Manual").map(str::to_string),
                    devices,
                    clips,
                    automation_lanes: automation_lanes.len(),
                    notes: notes.len(),
                },
            );
        }
    }

    model
}

fn extract_clips(chain: Option<&Element>) -> BTreeMap<String, Clip> {
    let mut clips = BTreeMap::new();
    let Some(chain) = chain else { return clips };

    let mut arranger_automations = Vec::new();
    chain.find_all_descendants("ArrangerAutomation", &mut arranger_automations);

    for aa in arranger_automations {
        let Some(events) = aa.find_child("Events") else {
            continue;
        };
        for clip_el in &events.children {
            let Some(cid) = clip_el.attr("Id") else {
                continue;
            };
            let sample_full = val(Some(clip_el), ".//SampleRef/FileRef/RelativePath").unwrap_or("");
            // Basename only — the full path differs between machines and
            // real sets embed other people's home directories.
            let sample = sample_full.rsplit('/').next().unwrap_or("").to_string();
            let name = val(Some(clip_el), "Name").unwrap_or("").to_string();
            let start = num(val(Some(clip_el), "CurrentStart")).unwrap_or(0.0);
            let end = num(val(Some(clip_el), "CurrentEnd")).unwrap_or(0.0);
            let disabled = val(Some(clip_el), "Disabled") == Some("true");
            clips.insert(
                cid.to_string(),
                Clip {
                    id: cid.to_string(),
                    name,
                    start,
                    end,
                    sample,
                    disabled,
                },
            );
        }
    }
    clips
}

fn extract_devices(chain: Option<&Element>) -> Vec<Device> {
    let mut devices = Vec::new();
    let Some(chain) = chain else { return devices };
    let mut containers = Vec::new();
    chain.find_all_descendants("Devices", &mut containers);
    for container in containers {
        for d in &container.children {
            devices.push(Device {
                tag: d.tag.clone(),
                fingerprint: device_fingerprint(d),
            });
        }
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::parse_xml;

    fn model_of(xml: &str) -> Model {
        let root = parse_xml(xml.as_bytes(), 256).unwrap();
        build_model(&root)
    }

    #[test]
    fn tempo_is_read_from_master_track() {
        let xml = r#"<Ableton><LiveSet><MasterTrack><DeviceChain><Mixer>
            <Tempo><Manual Value="128.0"/></Tempo>
        </Mixer></DeviceChain></MasterTrack></LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).tempo_bpm, Some(128.0));
    }

    #[test]
    fn tempo_is_read_from_main_track_live_12_3_bug_fix() {
        // M1 named bug fix 1: als_semantic_diff.py only checks MasterTrack.
        let xml = r#"<Ableton><LiveSet><MainTrack><DeviceChain><Mixer>
            <Tempo><Manual Value="128.0"/></Tempo>
        </Mixer></DeviceChain></MainTrack></LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).tempo_bpm, Some(128.0));
    }

    #[test]
    fn a_set_with_no_tempo_host_reports_none() {
        let xml = r#"<Ableton><LiveSet></LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).tempo_bpm, None);
    }

    #[test]
    fn only_whitelisted_track_tags_become_tracks() {
        let xml = r#"<Ableton><LiveSet><Tracks>
            <AudioTrack Id="1"><Name><EffectiveName Value="Real"/></Name></AudioTrack>
            <PreHearTrack Id="99"><Name><EffectiveName Value="cue"/></Name></PreHearTrack>
        </Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(model.tracks.len(), 1);
        assert!(model.tracks.contains_key(&TrackId::from("1")));
    }

    #[test]
    fn a_track_without_a_device_chain_does_not_crash() {
        let xml = r#"<Ableton><LiveSet><Tracks>
            <AudioTrack Id="1"><Name><EffectiveName Value="Odd"/></Name></AudioTrack>
        </Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        let track = &model.tracks[&TrackId::from("1")];
        assert_eq!(track.volume, None);
        assert!(track.clips.is_empty());
        assert!(track.devices.is_empty());
    }

    #[test]
    fn track_without_a_name_falls_back_to_a_placeholder() {
        let xml = r#"<Ableton><LiveSet><Tracks>
            <AudioTrack Id="1"></AudioTrack>
        </Tracks></LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).tracks[&TrackId::from("1")].name, "?");
    }

    #[test]
    fn mixer_values_are_rounded_to_three_places() {
        let xml = r#"<Ableton><LiveSet><Tracks>
            <AudioTrack Id="1"><DeviceChain><Mixer>
                <Volume><Manual Value="0.7943282127"/></Volume>
            </Mixer></DeviceChain></AudioTrack>
        </Tracks></LiveSet></Ableton>"#;
        assert_eq!(
            model_of(xml).tracks[&TrackId::from("1")].volume,
            Some(0.794)
        );
    }

    #[test]
    fn sample_reference_is_reduced_to_a_basename() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <MainSequencer><Sample><ArrangerAutomation><Events>
                <AudioClip Id="9">
                    <SampleRef><FileRef><RelativePath Value="Samples/Processed/Consolidate/take 3.wav"/></FileRef></SampleRef>
                </AudioClip>
            </Events></ArrangerAutomation></Sample></MainSequencer>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.tracks[&TrackId::from("1")].clips["9"].sample,
            "take 3.wav"
        );
    }

    #[test]
    fn device_chain_is_recorded_in_order() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <DeviceChain><Devices>
                <Eq8 Id="0"/><Compressor2 Id="1"/><AutoFilter Id="2"/>
            </Devices></DeviceChain>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.tracks[&TrackId::from("1")].device_tags(),
            vec!["Eq8", "Compressor2", "AutoFilter"]
        );
    }

    #[test]
    fn device_fingerprint_changes_when_a_manual_value_changes() {
        let make = |value: &str| {
            format!(
                r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
                <DeviceChain><Devices><Eq8 Id="0"><On><Manual Value="{value}"/></On></Eq8></Devices></DeviceChain>
            </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#
            )
        };
        let a = model_of(&make("1"));
        let b = model_of(&make("0"));
        let fp_a = a.tracks[&TrackId::from("1")].devices[0].fingerprint;
        let fp_b = b.tracks[&TrackId::from("1")].devices[0].fingerprint;
        assert_ne!(fp_a, fp_b);
    }

    #[test]
    fn device_fingerprint_is_stable_when_nothing_changes() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <DeviceChain><Devices><Eq8 Id="0"><On><Manual Value="1"/></On></Eq8></Devices></DeviceChain>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let a = model_of(xml);
        let b = model_of(xml);
        assert_eq!(
            a.tracks[&TrackId::from("1")].devices[0].fingerprint,
            b.tracks[&TrackId::from("1")].devices[0].fingerprint
        );
    }
}
