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
//!
//! ## PLAN-V2 additions (2026-09-29, "Phase C → Ableton")
//!
//! **Locators** (`LiveSet/Locators/Locators/Locator`) — **[measured]** on
//! the real 29-file Backup chain (`WIT_FIXTURES`): `<Locator Id="0">` holds
//! a `Time` (beats, the same unit as a clip's `CurrentStart`), a `Name`,
//! and `IsSongStart`. Wit reads name and time; it never reads `Id`, because
//! locator-id stability across saves has not been checked the way track-id
//! stability was (`docs/FORMATS.md`) — `wit-diff` matches locators by time
//! instead (see that crate).
//!
//! **Time signature** — the brief for this work assumed Live stores the
//! song's numerator/denominator directly on the main track's mixer. It
//! doesn't: `LiveSet/MainTrack/DeviceChain/Mixer/TimeSignature/Manual` is a
//! `RemoteableEnum` (confirmed against Live 12's own shipped schema,
//! `/Applications/Ableton Live 12 Suite.app/Contents/App-Resources/Schema/
//! 12.0_12402.txt`), and every save in the real 29-file chain carries the
//! same raw value (`201`) with nothing in the corpus to test a second value
//! against — decoding it would be a guess presented as a fact, which
//! AGENTS.md forbids. Instead, Wit reads the *explicit*
//! `Numerator`/`Denominator` Ableton attaches to a clip's own warp/grid
//! time-signature record (`.//TimeSignature/TimeSignatures/
//! RemoteableTimeSignature`, class `SingleTimeSignatureManager` per the
//! same schema) — every clip in the real chain agrees on `4/4`. This
//! mirrors a tested, MIT-licensed open-source parser
//! (github.com/owenbush/ableton-inspector, `time-signature.ts`, fetched
//! 2026-09-29), which reads the identical element for the same reason. The
//! model's time signature is therefore the earliest (`Time`-ordered) such
//! record in the file — **[inferred]** to represent the song's meter,
//! since Ableton does not publish a field that says so directly; `None`
//! when no clip carries one at all.
//!
//! **Key/scale** (`LiveSet/ScaleInformation` + the sibling `InKey` flag) —
//! **[measured]** that the element and the flag exist and that `InKey` is
//! `false` in every one of the 29 real files (root/name enums both `0`,
//! their inert default). Root-note and scale-name enum tables are
//! **[inferred]**/**[cited]** respectively — see `tables.rs` — and were
//! never exercised by real material, since nothing in the chain has the
//! scale feature on. `key` renders only when `InKey` is `true`.
//!
//! **Third-party plugin names** (`PluginDesc`) — **[measured]** on the
//! real chain: an `AuPluginDevice`'s `PluginDesc/AuPluginInfo` carries a
//! literal `Name`/`Manufacturer` (not `PlugName`, which a third-party
//! parser assumes — see `tables.rs`'s module doc). `VstPluginInfo`/
//! `Vst3PluginInfo` are read the same way but unverified: the real chain
//! has no VST plugins (its third-party plugin is an AU). Extraction keys
//! on the presence of `PluginDesc` itself, not on a device's own tag, so it
//! does not depend on knowing every third-party wrapper tag Live uses.
//!
//! **Device display names** — see `tables.rs`.
//!
//! **Master bus devices** — a real, reviewed save toggled a mastering
//! plugin off on the master bus, and Wit said nothing, because this
//! function only ever walked `LiveSet/Tracks`. Confirmed on the real chain:
//! `LiveSet/MainTrack/DeviceChain` nests `Mixer` and `DeviceChain/Devices`
//! exactly like a regular track's `DeviceChain` does, so the existing
//! [`extract_devices`] walk (already generic over where `Devices` sits)
//! reads it unchanged; only the wiring at the top of [`build_model`] is new.

use crate::dom::{find_path, val, Element};
use std::collections::BTreeMap;
use wit_model::{Clip, Device, Fingerprint, Locator, Model, Track, TrackId, TrackKind};

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
        time_signature: None,
        key: None,
        locators: Vec::new(),
        master_track_label: None,
        master_devices: Vec::new(),
    };

    let Some(live_set) = root.find_child("LiveSet") else {
        return model;
    };

    if let Some(tempo_el) = live_set.find_descendant("Tempo") {
        model.tempo_bpm = num(tempo_el.find_child("Manual").and_then(|m| m.attr("Value")));
    }

    model.time_signature = extract_time_signature(live_set);
    model.key = extract_key(live_set);
    model.locators = extract_locators(live_set);

    let (master_label, main_track) = live_set
        .find_child("MainTrack")
        .map(|t| ("Main", t))
        .or_else(|| live_set.find_child("MasterTrack").map(|t| ("Master", t)))
        .unzip();
    model.master_track_label = master_label.map(str::to_string);
    model.master_devices = extract_devices(main_track.and_then(|t| t.find_child("DeviceChain")));

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

/// The project's time signature, read from the earliest clip-level
/// `RemoteableTimeSignature` record in the whole `LiveSet` (see this
/// module's doc for why a clip's own record, not the enum-encoded one on
/// `MainTrack`/`MasterTrack`). Deterministic: [`Vec::sort_by`] is a stable
/// sort, so two records at the same `Time` keep document order.
fn extract_time_signature(live_set: &Element) -> Option<wit_model::TimeSignature> {
    let mut hosts = Vec::new();
    live_set.find_all_descendants("TimeSignature", &mut hosts);

    let mut candidates: Vec<(f64, u16, u16)> = Vec::new();
    for host in hosts {
        let Some(list) = host.find_child("TimeSignatures") else {
            continue; // the enum-encoded MainTrack/MasterTrack TimeSignature
        };
        for sig in list.find_all_children("RemoteableTimeSignature") {
            let numerator = sig
                .find_child("Numerator")
                .and_then(|e| e.attr("Value"))
                .and_then(|v| v.parse::<u16>().ok());
            let denominator = sig
                .find_child("Denominator")
                .and_then(|e| e.attr("Value"))
                .and_then(|v| v.parse::<u16>().ok());
            let Some((numerator, denominator)) = numerator.zip(denominator) else {
                continue;
            };
            if denominator == 0 {
                continue; // untrusted input: never divide by a zero denominator downstream
            }
            let time = sig
                .find_child("Time")
                .and_then(|e| e.attr("Value"))
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|t| t.is_finite())
                .unwrap_or(f64::INFINITY);
            candidates.push((time, numerator, denominator));
        }
    }
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    candidates
        .into_iter()
        .next()
        .map(|(_, numerator, denominator)| wit_model::TimeSignature {
            numerator,
            denominator,
        })
}

/// The project's key as "C minor", only when Ableton's own `InKey` flag
/// says the scale feature is actually on — see this module's doc for why
/// an unset scale and an unread one are the same `None` here.
fn extract_key(live_set: &Element) -> Option<String> {
    let scale = live_set.find_child("ScaleInformation")?;
    let in_key = live_set.find_child("InKey").and_then(|e| e.attr("Value"));
    if in_key != Some("true") {
        return None;
    }
    let root = scale
        .find_child("Root")
        .and_then(|e| e.attr("Value"))
        .and_then(|v| v.parse::<u32>().ok())?;
    let name = scale
        .find_child("Name")
        .and_then(|e| e.attr("Value"))
        .and_then(|v| v.parse::<u32>().ok())?;
    crate::tables::key_label(root, name)
}

/// Arrangement locators, in document order (Live's own order — see
/// `Locator`'s doc comment on `wit_model::Model`).
fn extract_locators(live_set: &Element) -> Vec<Locator> {
    let mut out = Vec::new();
    let Some(locators) = find_path(live_set, "Locators/Locators") else {
        return out;
    };
    for loc in locators.find_all_children("Locator") {
        let Some(time_beats) = loc
            .find_child("Time")
            .and_then(|e| e.attr("Value"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|t| t.is_finite())
        else {
            continue; // untrusted input: an unparseable or non-finite time is dropped, not guessed
        };
        let name = loc
            .find_child("Name")
            .and_then(|e| e.attr("Value"))
            .unwrap_or("")
            .to_string();
        out.push(Locator { name, time_beats });
    }
    out
}

/// A device's third-party plugin name, when it wraps one — keyed on the
/// presence of `PluginDesc` itself rather than on the device's own tag
/// (`AuPluginDevice` measured on the real chain; `Vst`/`Vst3PluginDevice`
/// assumed by naming symmetry but unverified — see this module's doc), so
/// this does not depend on enumerating every third-party wrapper tag Live
/// uses. `AuPluginInfo/Name` is what the real chain writes; a third-party
/// open-source parser assumes `PlugName` for every format, which measurably
/// does not match Live 12's AU shape, so both are tried.
fn plugin_name_of(device: &Element) -> Option<String> {
    let desc = device.find_child("PluginDesc")?;
    for host_tag in ["AuPluginInfo", "VstPluginInfo", "Vst3PluginInfo"] {
        let Some(info) = desc.find_child(host_tag) else {
            continue;
        };
        if let Some(name) = val(Some(info), "Name") {
            return Some(name.to_string());
        }
        if let Some(name) = val(Some(info), "PlugName") {
            return Some(name.to_string());
        }
    }
    None
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
                plugin_name: plugin_name_of(d),
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

    // ---- PLAN-V2: locators ------------------------------------------------

    #[test]
    fn locators_are_read_with_name_and_beat_time() {
        // Literal shape measured on the real 29-file Backup chain.
        let xml = r#"<Ableton><LiveSet>
            <Locators><Locators>
                <Locator Id="0"><Time Value="0"/><Name Value="Intro"/><Annotation Value=""/><IsSongStart Value="true"/></Locator>
                <Locator Id="1"><Time Value="32.5"/><Name Value="Chorus"/><Annotation Value=""/><IsSongStart Value="false"/></Locator>
            </Locators></Locators>
        </LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.locators,
            vec![
                wit_model::Locator {
                    name: "Intro".into(),
                    time_beats: 0.0
                },
                wit_model::Locator {
                    name: "Chorus".into(),
                    time_beats: 32.5
                },
            ]
        );
    }

    #[test]
    fn a_set_with_no_locators_reports_an_empty_list() {
        let model = model_of(r#"<Ableton><LiveSet></LiveSet></Ableton>"#);
        assert!(model.locators.is_empty());
    }

    // ---- PLAN-V2: time signature ------------------------------------------

    #[test]
    fn time_signature_is_read_from_a_clips_explicit_numerator_and_denominator() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <MainSequencer><Sample><ArrangerAutomation><Events>
                <AudioClip Id="9">
                    <TimeSignature><TimeSignatures>
                        <RemoteableTimeSignature Id="0"><Numerator Value="3"/><Denominator Value="4"/><Time Value="0"/></RemoteableTimeSignature>
                    </TimeSignatures></TimeSignature>
                </AudioClip>
            </Events></ArrangerAutomation></Sample></MainSequencer>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        assert_eq!(
            model_of(xml).time_signature,
            Some(wit_model::TimeSignature {
                numerator: 3,
                denominator: 4
            })
        );
    }

    #[test]
    fn time_signature_ignores_the_enum_encoded_main_track_manual_value() {
        // MainTrack/DeviceChain/Mixer/TimeSignature/Manual is a Live-internal
        // enum with no published decode table (this module's doc) — it must
        // never be mistaken for a literal numerator/denominator.
        let xml = r#"<Ableton><LiveSet>
            <MainTrack><DeviceChain><Mixer>
                <TimeSignature><Manual Value="201"/></TimeSignature>
            </Mixer></DeviceChain></MainTrack>
        </LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).time_signature, None);
    }

    #[test]
    fn the_earliest_time_signature_record_wins_deterministically() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <MainSequencer><Sample><ArrangerAutomation><Events>
                <AudioClip Id="9">
                    <TimeSignature><TimeSignatures>
                        <RemoteableTimeSignature Id="0"><Numerator Value="7"/><Denominator Value="8"/><Time Value="16"/></RemoteableTimeSignature>
                    </TimeSignatures></TimeSignature>
                </AudioClip>
                <AudioClip Id="10">
                    <TimeSignature><TimeSignatures>
                        <RemoteableTimeSignature Id="0"><Numerator Value="4"/><Denominator Value="4"/><Time Value="0"/></RemoteableTimeSignature>
                    </TimeSignatures></TimeSignature>
                </AudioClip>
            </Events></ArrangerAutomation></Sample></MainSequencer>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        assert_eq!(
            model_of(xml).time_signature,
            Some(wit_model::TimeSignature {
                numerator: 4,
                denominator: 4
            })
        );
    }

    #[test]
    fn a_zero_denominator_in_untrusted_input_is_dropped_not_divided_by() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <MainSequencer><Sample><ArrangerAutomation><Events>
                <AudioClip Id="9">
                    <TimeSignature><TimeSignatures>
                        <RemoteableTimeSignature Id="0"><Numerator Value="4"/><Denominator Value="0"/><Time Value="0"/></RemoteableTimeSignature>
                    </TimeSignatures></TimeSignature>
                </AudioClip>
            </Events></ArrangerAutomation></Sample></MainSequencer>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).time_signature, None);
    }

    // ---- PLAN-V2: key/scale ------------------------------------------------

    #[test]
    fn key_is_read_only_when_in_key_is_true() {
        let xml = r#"<Ableton><LiveSet>
            <ScaleInformation><Root Value="0"/><Name Value="1"/></ScaleInformation>
            <InKey Value="true"/>
        </LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).key.as_deref(), Some("C minor"));
    }

    #[test]
    fn key_is_none_when_in_key_is_false_even_with_scale_information_present() {
        // Measured on the real 29-file Backup chain: every file has
        // ScaleInformation Root=0/Name=0 but InKey=false.
        let xml = r#"<Ableton><LiveSet>
            <ScaleInformation><Root Value="0"/><Name Value="0"/></ScaleInformation>
            <InKey Value="false"/>
        </LiveSet></Ableton>"#;
        assert_eq!(model_of(xml).key, None);
    }

    #[test]
    fn key_is_none_when_scale_information_is_absent() {
        assert_eq!(
            model_of(r#"<Ableton><LiveSet></LiveSet></Ableton>"#).key,
            None
        );
    }

    // ---- PLAN-V2: third-party plugin names --------------------------------

    #[test]
    fn a_third_party_au_plugin_name_is_read_from_plugin_desc() {
        // Literal shape measured on the real 29-file Backup chain: Name and
        // Manufacturer, not PlugName.
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <DeviceChain><Devices>
                <AuPluginDevice Id="0"><PluginDesc><AuPluginInfo Id="0">
                    <Name Value="Pro-Q 3"/><Manufacturer Value="FabFilter"/>
                </AuPluginInfo></PluginDesc></AuPluginDevice>
            </Devices></DeviceChain>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.tracks[&TrackId::from("1")].devices[0]
                .plugin_name
                .as_deref(),
            Some("Pro-Q 3")
        );
    }

    #[test]
    fn a_native_device_has_no_plugin_name() {
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <DeviceChain><Devices><Eq8 Id="0"/></Devices></DeviceChain>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.tracks[&TrackId::from("1")].devices[0].plugin_name,
            None
        );
    }

    #[test]
    fn a_vst_plugin_name_falls_back_to_the_plug_name_element() {
        // Unverified against the real chain (it has no VST plugins) — see
        // this module's doc — but a device with a VstPluginInfo lacking
        // `Name` must still resolve via `PlugName` rather than silently
        // reporting no plugin at all.
        let xml = r#"<Ableton><LiveSet><Tracks><AudioTrack Id="1"><DeviceChain>
            <DeviceChain><Devices>
                <PluginDevice Id="0"><PluginDesc><VstPluginInfo Id="0">
                    <PlugName Value="Serum"/>
                </VstPluginInfo></PluginDesc></PluginDevice>
            </Devices></DeviceChain>
        </DeviceChain></AudioTrack></Tracks></LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(
            model.tracks[&TrackId::from("1")].devices[0]
                .plugin_name
                .as_deref(),
            Some("Serum")
        );
    }

    // ---- PLAN-V2: master bus devices --------------------------------------

    #[test]
    fn main_track_devices_are_read_and_labelled_main() {
        let xml = r#"<Ableton><LiveSet>
            <MainTrack><DeviceChain>
                <Mixer><Tempo><Manual Value="120.0"/></Tempo></Mixer>
                <DeviceChain><Devices>
                    <AuPluginDevice Id="3"><On><Manual Value="true"/></On><PluginDesc><AuPluginInfo Id="0">
                        <Name Value="Ozone"/><Manufacturer Value="iZotope"/>
                    </AuPluginInfo></PluginDesc></AuPluginDevice>
                </Devices></DeviceChain>
            </DeviceChain></MainTrack>
        </LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(model.master_track_label.as_deref(), Some("Main"));
        assert_eq!(model.master_devices.len(), 1);
        assert_eq!(model.master_devices[0].tag, "AuPluginDevice");
        assert_eq!(
            model.master_devices[0].plugin_name.as_deref(),
            Some("Ozone")
        );
    }

    #[test]
    fn a_legacy_master_track_is_labelled_master() {
        let xml = r#"<Ableton><LiveSet>
            <MasterTrack><DeviceChain><Mixer><Tempo><Manual Value="120.0"/></Tempo></Mixer></DeviceChain></MasterTrack>
        </LiveSet></Ableton>"#;
        let model = model_of(xml);
        assert_eq!(model.master_track_label.as_deref(), Some("Master"));
        assert!(model.master_devices.is_empty());
    }

    #[test]
    fn toggling_a_master_bus_plugin_off_changes_its_fingerprint() {
        // The exact real-world bug this whole feature fixes: a save that
        // only flips a master-bus plugin's On/Manual value must not look
        // identical to Wit.
        let make = |on: &str| {
            format!(
                r#"<Ableton><LiveSet><MainTrack><DeviceChain><DeviceChain><Devices>
                    <AuPluginDevice Id="3"><On><Manual Value="{on}"/></On></AuPluginDevice>
                </Devices></DeviceChain></DeviceChain></MainTrack></LiveSet></Ableton>"#
            )
        };
        let on = model_of(&make("true"));
        let off = model_of(&make("false"));
        assert_ne!(
            on.master_devices[0].fingerprint,
            off.master_devices[0].fingerprint
        );
    }

    #[test]
    fn a_set_with_no_main_or_master_track_reports_no_label() {
        let model = model_of(r#"<Ableton><LiveSet></LiveSet></Ableton>"#);
        assert_eq!(model.master_track_label, None);
        assert!(model.master_devices.is_empty());
    }
}
