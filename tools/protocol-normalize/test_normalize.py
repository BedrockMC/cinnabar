"""Neutral synthetic mutations exercise selectors without retaining source aliases."""
import json
from pathlib import Path
import unittest

import normalize as normalizer


class NormalizationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads(Path(__file__).with_name("manifest.json").read_text())
        root = Path(__file__).resolve().parents[2]
        cls.input = {name: (root / normalizer.SOURCE / name).read_bytes() for name in normalizer.FILES}
        cls.canonical = normalizer.normalize(cls.input, cls.manifest)

    def test_checked_in_sources_are_canonical(self):
        self.assertEqual({name: normalizer.digest(value) for name, value in self.input.items()},
                         {name: normalizer.digest(value) for name, value in self.canonical.items()})

    def test_second_run_is_byte_identical(self):
        self.assertEqual(normalizer.normalize(self.canonical, self.manifest), self.canonical)

    def test_numeric_extension_closure_is_canonical(self):
        text = self.input["types.rs"].decode()
        self.assertTrue("EnumsLegacyTelemetryEventPacketPayloadType::Reserved26" in text)
        self.assertTrue("LegacyTelemetryEventPacketEventData::Reserved18" in text)
        self.assertTrue("LegacyTelemetryEventPacketEventData::Reserved19" in text)
        self.assertTrue(b"pub struct ReservedPacket65Event18View" in self.input["borrowed.rs"])

    def test_codec_literal_drift_is_rejected(self):
        data = dict(self.canonical)
        data["common.rs"] = data["common.rs"].replace(b"109u32", b"110u32", 1)
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest)

    def test_mixed_source_states_are_rejected(self):
        data = dict(self.canonical)
        data["common.rs"] = data["common.rs"].replace(b"ReservedPacket109", b"SyntheticPacket109")
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest)

    def test_neutral_collision_is_rejected(self):
        data = dict(self.canonical)
        data["proto.rs"] += b"\npub struct ReservedPacket109 { pub duplicate: u8, }\n"
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest, False)

    def test_borrowed_correspondence_drift_is_rejected(self):
        data = dict(self.canonical)
        data["borrowed.rs"] = data["borrowed.rs"].replace(b"\r\n", b"\n").replace(
            b"pub struct ReservedPacket109View {\n    pub reserved_field_0:",
            b"pub struct ReservedPacket109View {\n    pub synthetic_field:")
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest, False)

    def test_unreviewed_shared_reference_is_rejected(self):
        data = dict(self.canonical)
        data["types.rs"] += b"\npub struct SyntheticRetailRecord {\n    pub value: ReservedPacket170Field0,\n}\n"
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest, False)

    def test_unreviewed_enum_reference_is_rejected(self):
        data = dict(self.canonical)
        data["types.rs"] += b"\npub enum SyntheticRetailUnion {\n    Value(ReservedPacket170Field0),\n}\n"
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(data, self.manifest, False)

    def test_action_union_discriminator_is_independent(self):
        manifest = dict(self.manifest, action_union_discriminator=9)
        with self.assertRaises(normalizer.ShapeError):
            normalizer.normalize(self.canonical, manifest, False)

    def test_only_identifiers_and_exact_debug_labels_are_rewritten(self):
        text = 'pub struct Synthetic {\n pub value: u8,\n}\nimpl Synthetic { fn f() { let value = 1; panic!("value"); }}\n'
        result = normalizer.rewrite_source(text, {"Synthetic": "Reserved"}, {"Synthetic": {"value": "reserved_field_0"}})
        self.assertIn('panic!("value")', result)
        self.assertIn("let reserved_field_0 = 1", result)
        self.assertIn("pub struct Reserved", result)


if __name__ == "__main__":
    unittest.main()
