"""
Synthetic Ableton Live Set (``.als``) factory.

WHY THIS EXISTS
    ``.als`` files are DAW project files. Wit must never commit one (CI enforces it,
    see ``.github/workflows/ci.yml``), and real sets embed other people's absolute
    home directories and sample names. So every fixture the test suite uses is built
    here, in code, from a declarative description.

WHAT IT PRODUCES
    Gzipped XML with the element paths verified against a real Live 12.3.5 set:

        Ableton[@Creator]
          LiveSet
            Tracks
              AudioTrack|MidiTrack|GroupTrack|ReturnTrack [@Id]
                Name/EffectiveName[@Value]
                Color[@Value]
                AutomationEnvelopes/Envelopes/AutomationEnvelope*
                DeviceChain
                  Mixer/{Volume,Pan,Speaker}/Manual[@Value]
                  DeviceChain/Devices/<DeviceTag>*
                  MainSequencer/Sample/ArrangerAutomation/Events/AudioClip[@Id]
                    CurrentStart, CurrentEnd, Name, Disabled
                    SampleRef/FileRef/RelativePath[@Value]
            MasterTrack/DeviceChain/Mixer/Tempo/Manual[@Value]

    Real Live 12.3 writes ``MainTrack`` rather than ``MasterTrack`` for the last
    element; ``LiveSet.tempo_host_tag`` lets a test choose, which is how the
    tempo-detection bug is pinned down (see test_als_model.py).

DESIGN NOTES
    - Output is byte-deterministic: gzip mtime is forced to 0, so the same
      description always produces the same file. Tests can compare bytes.
    - Realistic *noise* (view state, warp markers, absolute Path, OriginalCrc,
      LomId ...) is included on purpose. Without it a "view-state only change"
      test proves nothing, because there would be no view state to change.
    - Every value the semantic model reads is settable; nothing is hard-coded to a
      value a test also asserts on.
"""

from __future__ import annotations

import copy
import gzip
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from typing import List

__all__ = [
    "Clip",
    "Track",
    "LiveSet",
    "simple_set",
    "write_als",
    "als_bytes",
    "als_xml_bytes",
]

TRACK_KINDS = ("AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack")


# --------------------------------------------------------------------------- #
# description objects
# --------------------------------------------------------------------------- #


@dataclass
class Clip:
    """One clip on the arrangement timeline."""

    id: str
    name: str = ""
    start: float = 0.0
    end: float = 4.0
    sample: str = ""  # RelativePath value, e.g. "Samples/Imported/kick.wav"
    disabled: bool = False
    notes: int = 0  # MidiNoteEvent count (MidiTrack clips only)
    warp_markers: int = 2  # structural noise, ignored by the semantic model
    scroller_time: float = 0.0  # view state, ignored by the semantic model


@dataclass
class Track:
    """One track: mixer state, device chain, clips."""

    id: str
    name: str
    kind: str = "AudioTrack"
    color: str = "13"
    volume: float = 1.0
    pan: float = 0.0
    speaker: bool = True
    devices: List[str] = field(default_factory=list)
    clips: List[Clip] = field(default_factory=list)
    automation_lanes: int = 0
    selected: bool = False  # view state

    def clip(self, clip_id: str) -> Clip:
        for c in self.clips:
            if c.id == clip_id:
                return c
        raise KeyError("no clip with Id=%r on track %r" % (clip_id, self.name))


@dataclass
class LiveSet:
    """A whole Live set."""

    tracks: List[Track] = field(default_factory=list)
    tempo: float = 120.0
    creator: str = "Ableton Live 12.0.5"
    # Live <= 12.2 writes MasterTrack; 12.3 writes MainTrack. See test_als_model.py.
    tempo_host_tag: str = "MasterTrack"
    # view state — must never influence the semantic model
    scroll_x: int = 0
    zoom: float = 1.0
    selected_track: str = ""
    next_pointee_id: int = 40000

    # -- convenience -------------------------------------------------------- #

    def copy(self) -> "LiveSet":
        return copy.deepcopy(self)

    def track(self, name_or_id: str) -> Track:
        for t in self.tracks:
            if t.name == name_or_id or t.id == name_or_id:
                return t
        raise KeyError("no track named or numbered %r" % (name_or_id,))

    def remove_track(self, name_or_id: str) -> Track:
        t = self.track(name_or_id)
        self.tracks.remove(t)
        return t

    # -- serialisation ------------------------------------------------------ #

    def to_xml_bytes(self) -> bytes:
        return als_xml_bytes(self)

    def to_bytes(self) -> bytes:
        return als_bytes(self)

    def write(self, path) -> str:
        return write_als(self, path)


# --------------------------------------------------------------------------- #
# XML emission
# --------------------------------------------------------------------------- #


def _fmt(value) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _v(parent: ET.Element, tag: str, value) -> ET.Element:
    el = ET.SubElement(parent, tag)
    el.set("Value", _fmt(value))
    return el


def _lom(parent: ET.Element) -> None:
    """Bookkeeping elements every real Live element carries."""
    _v(parent, "LomId", 0)
    _v(parent, "LomIdView", 0)


def _manual_param(parent: ET.Element, tag: str, value, target_id: int) -> ET.Element:
    """A Live automatable parameter: <Tag><Manual Value=.../>...</Tag>."""
    el = ET.SubElement(parent, tag)
    _v(el, "LomId", 0)
    _v(el, "Manual", value)
    mr = ET.SubElement(el, "MidiControllerRange")
    _v(mr, "Min", 0)
    _v(mr, "Max", 1)
    ET.SubElement(el, "AutomationTarget", {"Id": str(target_id)})
    return el


def _sample_ref(clip_el: ET.Element, relative_path: str) -> None:
    ref = ET.SubElement(clip_el, "SampleRef")
    fref = ET.SubElement(ref, "FileRef")
    _v(fref, "RelativePathType", 3)
    _v(fref, "RelativePath", relative_path)
    # Real sets store an absolute path here — often somebody else's home directory.
    # The synthetic one is deliberately anonymous.
    _v(fref, "Path", "/Users/synthetic/Wit Test Project/" + relative_path)
    _v(fref, "Type", 2)
    _v(fref, "LivePackName", "")
    _v(fref, "OriginalFileSize", 1234567)
    _v(fref, "OriginalCrc", 4242)
    _v(ref, "LastModDate", 1771910201)
    _v(ref, "SampleUsageHint", 0)
    _v(ref, "DefaultDuration", 220500)
    _v(ref, "DefaultSampleRate", 44100)


def _clip_element(events: ET.Element, clip: Clip, kind: str, pid: List[int]) -> None:
    tag = "MidiClip" if kind == "MidiTrack" else "AudioClip"
    el = ET.SubElement(events, tag, {"Id": clip.id, "Time": _fmt(clip.start)})
    _lom(el)
    _v(el, "CurrentStart", clip.start)
    _v(el, "CurrentEnd", clip.end)
    loop = ET.SubElement(el, "Loop")
    _v(loop, "LoopStart", 0)
    _v(loop, "LoopEnd", clip.end - clip.start)
    _v(loop, "LoopOn", False)
    _v(el, "Name", clip.name)
    _v(el, "Annotation", "")
    _v(el, "Color", 13)
    _v(el, "Disabled", clip.disabled)
    # view state — the model must ignore this
    _v(el, "ScrollerTimePreserve", clip.scroller_time)

    if tag == "AudioClip":
        wm = ET.SubElement(el, "WarpMarkers")
        for i in range(clip.warp_markers):
            ET.SubElement(
                wm,
                "WarpMarker",
                {"Id": str(i), "SecTime": str(float(i)), "BeatTime": str(float(i * 2))},
            )
        _v(el, "WarpMode", 0)
        if clip.sample:
            _sample_ref(el, clip.sample)
    else:
        notes = ET.SubElement(el, "Notes")
        ktracks = ET.SubElement(notes, "KeyTracks")
        kt = ET.SubElement(ktracks, "KeyTrack", {"Id": "0"})
        inner = ET.SubElement(kt, "Notes")
        for i in range(clip.notes):
            ET.SubElement(
                inner,
                "MidiNoteEvent",
                {
                    "Time": str(float(i)),
                    "Duration": "0.5",
                    "Velocity": "100",
                    "IsEnabled": "true",
                },
            )
        _v(kt, "MidiKey", 60)
    pid[0] += 1


def _track_element(tracks_el: ET.Element, track: Track, pid: List[int]) -> None:
    if track.kind not in TRACK_KINDS:
        raise ValueError("unknown track kind %r" % (track.kind,))
    tr = ET.SubElement(
        tracks_el,
        track.kind,
        {"Id": track.id, "SelectedToolPanel": "3", "SelectedTransformationName": ""},
    )
    _lom(tr)
    _v(tr, "IsContentSelectedInDocument", track.selected)
    delay = ET.SubElement(tr, "TrackDelay")
    _v(delay, "Value", 0)
    _v(delay, "IsValueSampleBased", False)
    name = ET.SubElement(tr, "Name")
    _v(name, "EffectiveName", track.name)
    _v(name, "UserName", track.name)
    _v(name, "Annotation", "")
    _v(tr, "Color", track.color)

    envs_outer = ET.SubElement(tr, "AutomationEnvelopes")
    envs = ET.SubElement(envs_outer, "Envelopes")
    for i in range(track.automation_lanes):
        env = ET.SubElement(envs, "AutomationEnvelope", {"Id": str(i)})
        target = ET.SubElement(env, "EnvelopeTarget")
        _v(target, "PointeeId", 1000 + i)

    chain = ET.SubElement(tr, "DeviceChain")

    routing = ET.SubElement(chain, "AudioOutputRouting")
    _v(routing, "Target", "AudioOut/Main")
    _v(routing, "UpperDisplayString", "Main")

    mixer = ET.SubElement(chain, "Mixer")
    _lom(mixer)
    _v(mixer, "IsExpanded", True)
    _manual_param(mixer, "On", True, pid[0] + 1)
    _manual_param(mixer, "Speaker", track.speaker, pid[0] + 2)
    _manual_param(mixer, "Volume", track.volume, pid[0] + 3)
    _manual_param(mixer, "Pan", track.pan, pid[0] + 4)
    pid[0] += 8

    inner_chain = ET.SubElement(chain, "DeviceChain")
    devices = ET.SubElement(inner_chain, "Devices")
    for i, dev in enumerate(track.devices):
        d = ET.SubElement(devices, dev, {"Id": str(i)})
        _lom(d)
        _manual_param(d, "On", True, pid[0] + i)
    pid[0] += max(len(track.devices), 1)
    ET.SubElement(inner_chain, "SignalModulations")

    seq = ET.SubElement(chain, "MainSequencer")
    _lom(seq)
    holder = ET.SubElement(seq, "ClipTimeable" if track.kind == "MidiTrack" else "Sample")
    arranger = ET.SubElement(holder, "ArrangerAutomation")
    events = ET.SubElement(arranger, "Events")
    for clip in track.clips:
        _clip_element(events, clip, track.kind, pid)
    _v(arranger, "AutomationTransformViewState", 0)


def _tempo_host(live_set: ET.Element, ls: LiveSet) -> None:
    host = ET.SubElement(live_set, ls.tempo_host_tag)
    _lom(host)
    name = ET.SubElement(host, "Name")
    _v(name, "EffectiveName", "Master")
    chain = ET.SubElement(host, "DeviceChain")
    mixer = ET.SubElement(chain, "Mixer")
    _manual_param(mixer, "Tempo", ls.tempo, 8)
    _manual_param(mixer, "Volume", 1.0, 9)


def als_xml_bytes(ls: LiveSet) -> bytes:
    """Serialise to uncompressed XML bytes (what ``gunzip -c`` would print)."""
    root = ET.Element(
        "Ableton",
        {
            "MajorVersion": "5",
            "MinorVersion": "12.0_12049",
            "SchemaChangeCount": "3",
            "Creator": ls.creator,
            "Revision": "0" * 40,
        },
    )
    live_set = ET.SubElement(root, "LiveSet")
    _v(live_set, "NextPointeeId", ls.next_pointee_id)
    _v(live_set, "OverwriteProtectionNumber", 3075)
    _lom(live_set)

    tracks_el = ET.SubElement(live_set, "Tracks")
    pid = [100]
    for track in ls.tracks:
        _track_element(tracks_el, track, pid)

    _tempo_host(live_set, ls)

    # ------------------------------------------------------------------ #
    # View state. Present so that "only the view changed" is a real case.
    # ------------------------------------------------------------------ #
    views = ET.SubElement(live_set, "ViewStates")
    _v(views, "SessionIO", 0)
    _v(views, "ArrangerScrollPosition", ls.scroll_x)
    _v(views, "ArrangerZoom", ls.zoom)
    _v(views, "SelectedTrack", ls.selected_track)
    _v(live_set, "ScrollerTimePreserver", ls.scroll_x)

    body = ET.tostring(root, encoding="utf-8")
    return b'<?xml version="1.0" encoding="UTF-8"?>\n' + body + b"\n"


def als_bytes(ls: LiveSet) -> bytes:
    """Serialise to gzipped bytes — a real ``.als`` payload, byte-deterministic."""
    import io

    buf = io.BytesIO()
    # mtime=0 so identical descriptions produce identical files.
    with gzip.GzipFile(fileobj=buf, mode="wb", compresslevel=6, mtime=0) as fh:
        fh.write(als_xml_bytes(ls))
    return buf.getvalue()


def write_als(ls: LiveSet, path) -> str:
    path = str(path)
    with open(path, "wb") as fh:
        fh.write(als_bytes(ls))
    return path


# --------------------------------------------------------------------------- #
# a reasonable default set
# --------------------------------------------------------------------------- #


def simple_set(**overrides) -> LiveSet:
    """
    A three-track set that exercises every field the semantic model reads.

    Track 100 'Drums'  — audio, 2 clips, one device
    Track 101 'Bass'   — audio, 1 clip, no devices, non-default volume/pan
    Track 102 'Keys'   — midi, 1 clip with notes, 1 automation lane
    """
    ls = LiveSet(
        tracks=[
            Track(
                id="100",
                name="Drums",
                kind="AudioTrack",
                color="13",
                volume=1.0,
                pan=0.0,
                devices=["Eq8"],
                clips=[
                    Clip(id="10", name="kick loop", start=0.0, end=16.0,
                         sample="Samples/Imported/kick.wav"),
                    Clip(id="11", name="hat loop", start=16.0, end=32.0,
                         sample="Samples/Imported/hat.wav"),
                ],
            ),
            Track(
                id="101",
                name="Bass",
                kind="AudioTrack",
                color="26",
                volume=0.794,
                pan=-0.25,
                clips=[
                    Clip(id="20", name="sub", start=0.0, end=32.0,
                         sample="Samples/Recorded/sub.wav"),
                ],
            ),
            Track(
                id="102",
                name="Keys",
                kind="MidiTrack",
                color="60",
                volume=0.85,
                automation_lanes=1,
                clips=[Clip(id="30", name="chords", start=8.0, end=24.0, notes=12)],
            ),
        ]
    )
    for key, value in overrides.items():
        setattr(ls, key, value)
    return ls
