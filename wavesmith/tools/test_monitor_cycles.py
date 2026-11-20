from contextlib import nullcontext
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from monitor_cycles import baseline_revision, decode, main, summarize, IDENTIFIER, FIELDS, WIDTHS


def packet(**changes):
    values = dict(instance=0,sequence=0,display_cycles=12000,total_cycles=24000000,callbacks=2000,
                  frames=48000,sample_rate=48000,block_frames=24,voices=10,midi=0,stereo=1)
    values.update(changes)
    data = list(IDENTIFIER + (1,))
    for key,width in zip(FIELDS,WIDTHS):
        data.extend((values[key] >> (7*i)) & 127 for i in range(width))
    return data


def revision_packet(revision, **changes):
    data = packet(**changes)
    data[4] = revision
    data.extend((24000000 >> (7 * i)) & 127 for i in range(10))
    data.extend([0] * ({5: 168, 6: 293}[revision] - len(data)))
    return data


class MonitorTests(unittest.TestCase):
    def test_decode_roundtrip_and_weighted_mean(self):
        first = decode(packet(total_cycles=25000000))
        second = decode(packet(sequence=1,total_cycles=12000000,callbacks=1000))
        self.assertEqual(first["cycles_per_callback"],12500)
        result = summarize([first,second],"test")
        self.assertAlmostEqual(result["mean_cycles_per_callback"],37000000/3000)
        self.assertEqual(result["missed_reports"],0)
        self.assertEqual(result["telemetry_revision"], 1)

    def test_legacy_baselines_infer_revision_only_when_identifiable(self):
        for revision in (5, 6):
            rows = [decode(revision_packet(revision, sequence=n)) for n in (0, 1)]
            baseline = summarize(rows, "legacy")
            self.assertEqual(baseline.pop("telemetry_revision"), revision)
            self.assertEqual(baseline_revision(baseline), revision)
        ambiguous = summarize([decode(revision_packet(5))], "one report")
        del ambiguous["telemetry_revision"]
        with self.assertRaisesRegex(ValueError, "Capture a fresh baseline"):
            baseline_revision(ambiguous)

    def test_compare_checks_revision_on_first_report(self):
        for measured, baseline in ((6, 5), (5, 6), (5, 5), (6, 6)):
            with self.subTest(measured=measured, baseline=baseline), TemporaryDirectory() as directory:
                path = Path(directory)
                baseline_path = path / "baseline.json"
                reports = [decode(revision_packet(baseline, sequence=n)) for n in (0, 1)]
                baseline_data = summarize(reports, "baseline")
                del baseline_data["telemetry_revision"]
                baseline_path.write_text(json.dumps(baseline_data), encoding="utf-8")
                sent = []
                message = SimpleNamespace(type="sysex", data=revision_packet(measured))
                mido = SimpleNamespace(
                    get_input_names=lambda: ["disting NT"],
                    get_output_names=lambda: ["disting NT"],
                    open_input=lambda _: nullcontext(SimpleNamespace(iter_pending=lambda: [message])),
                    open_output=lambda _: nullcontext(SimpleNamespace(send=sent.append)),
                    Message=lambda _, *, data: data,
                )
                output = path / "result.csv"
                args = ["--compare", str(baseline_path), "--output", str(output), "--seconds", "1", "--warmup", "0"]
                with patch.dict("sys.modules", {"mido": mido}), patch("monitor_cycles.time.sleep"), \
                     patch("monitor_cycles.time.monotonic", side_effect=[0, 2]), patch("builtins.print"):
                    if measured != baseline:
                        with self.assertRaisesRegex(ValueError, f"revision {measured} with baseline revision {baseline}"):
                            main(args)
                        self.assertFalse(output.with_suffix(".json").exists())
                        self.assertEqual(len(output.read_text(encoding="utf-8").splitlines()), 1)
                    else:
                        self.assertEqual(main(args), 0)
                        result = json.loads(output.with_suffix(".json").read_text(encoding="utf-8"))
                        self.assertEqual(result["telemetry_revision"], measured)
                        self.assertEqual(result["change_percent"], 0)
                self.assertEqual(sent, [IDENTIFIER + (0x10,), IDENTIFIER + (0x11,)])

    def test_supports_full_voice_count(self):
        for voices in [0, 10, 15, 24]:
            self.assertEqual(decode(packet(voices=voices))["voices"], voices)
        with self.assertRaises(ValueError):
            decode(packet(voices=25))

    def test_reject_malformed_and_changed_workload(self):
        self.assertIsNone(decode([0,33,39]))
        with self.assertRaises(ValueError): decode(packet()[:-1])
        with self.assertRaises(ValueError): decode(packet(callbacks=0))
        with self.assertRaises(ValueError): summarize([decode(packet()),decode(packet(sequence=1,voices=9))],"test")
        with self.assertRaises(ValueError): summarize([decode(packet()),decode(packet())],"test")

    def test_mixed_workload_retains_snapshots_and_rejects_instance_or_clock_changes(self):
        first = decode(packet())
        second = decode(packet(sequence=1, voices=3, midi=1, stereo=0))
        result = summarize([first, second], "playing", allow_workload_changes=True)
        self.assertTrue(result["mixed_workload"])
        self.assertEqual(result["device_seconds"], 2)
        self.assertEqual(result["workload"], dict(instance=0, sample_rate=48000, block_frames=24))
        self.assertEqual([row["voices"] for row in result["end_of_window_workload_snapshots"]], [10, 3])
        self.assertEqual(result["mean_cycles_per_callback"], 12000)
        for field in ("instance", "sample_rate", "block_frames"):
            with self.assertRaises(ValueError):
                summarize([first, decode(packet(sequence=1, **{field: first[field] + 1}))],
                          "invalid", allow_workload_changes=True)

    def test_extended_stage_totals(self):
        data = packet()
        data[4] = 2
        for value in [40000000,3000000,4000000,5000000,6000000]:
            data.extend((value >> (7*i)) & 127 for i in range(10))
        row = decode(data)
        result = summarize([row], "extended")
        self.assertEqual(result["mean_callback_cycles"],20000)
        self.assertEqual(result["mean_setup_cycles"],1500)
        self.assertEqual(result["mean_output_cycles"],3000)
        with self.assertRaises(ValueError):
            summarize([decode(packet()), row], "mixed")
        data[-1] = 127
        with self.assertRaises(ValueError):
            decode(data)


    def test_draw_counters_wrap_and_use_draw_count(self):
        def draw_packet(sequence, cycles, calls):
            data = packet(sequence=sequence)
            data[4] = 3
            for value in [40000000, 0, 0, 0, 0]:
                data.extend((value >> (7*i)) & 127 for i in range(10))
            for value in [cycles, calls]:
                data.extend((value >> (7*i)) & 127 for i in range(5))
            return decode(data)
        result = summarize([draw_packet(0, 0xfffffff0, 10), draw_packet(1, 984, 12)], "draw")
        self.assertEqual(result["mean_draw_cycles"], 500)
        self.assertEqual(result["draw_calls_per_second"], 2)

    def test_ui_stats_separate_idle_edits_and_setup(self):
        def ui_packet(sequence, counters):
            data = packet(sequence=sequence)
            data[4] = 4
            data.extend([0] * 50)
            data.extend([0] * 10)
            for value in counters:
                data.extend((value >> (7*i)) & 127 for i in range(5))
            return decode(data)
        rows = [ui_packet(0, [0]*9), ui_packet(1, [1000, 10, 200, 600, 2, 400, 0, 0, 0])]
        summary = summarize(rows, "ui")
        self.assertEqual(summary["ui_idle_mean_cycles"], 100)
        self.assertEqual(summary["ui_edit_mean_cycles"], 300)
        self.assertEqual(summary["ui_edit_max_cycles_since_instantiation"], 400)
        self.assertIsNone(summary["ui_setup_mean_cycles"])
        self.assertEqual(summary["ui_setup_calls"], 0)

    def test_label_callback_counts_and_wrap(self):
        def labels(sequence, string_calls, prefix_calls):
            data = packet(sequence=sequence)
            data[4] = 5
            data.extend([0] * 105)
            for value in [string_calls, prefix_calls]:
                data.extend((value >> (7*i)) & 127 for i in range(5))
            return decode(data)
        rows = [labels(0, 0xfffffffe, 50), labels(1, 3, 60)]
        summary = summarize(rows, "labels")
        self.assertEqual(summary["parameter_string_calls"], 5)
        self.assertEqual(summary["prefix_calls"], 10)
        self.assertEqual(summary["prefix_calls_per_second"], 10)
        self.assertEqual(summary["prefix_calls_at_first_report"], 50)

    def test_render_details_round_trip_reset_and_summary(self):
        def details(sequence):
            data = packet(sequence=sequence)
            data[4] = 6
            data.extend([0]*115)
            for values in [(600,3,100,300), (0,0,0,0), (80,2,40,40), (50,1,50,50), (30,3,10,10)]:
                for value,width in zip(values,(10,5,5,5)):
                    data.extend((value>>(7*i))&127 for i in range(width))
            return data
        row = decode(details(0))
        summary = summarize([row,decode(details(1))], "render")
        metric = summary["render_breakdown"]["group_gates"]
        self.assertEqual(metric["total_cycles"],1200)
        self.assertEqual(metric["calls"],6)
        self.assertEqual(metric["mean_cycles_per_call"],200)
        self.assertEqual(metric["min_cycles"],100)
        self.assertEqual(metric["max_cycles"],300)
        self.assertEqual(metric["mean_cycles_per_callback"],0.3)
        self.assertIsNone(summary["render_breakdown"]["open_playback"]["mean_cycles_per_call"])
        bad = details(0)
        bad[-1] = 127
        with self.assertRaises(ValueError): decode(bad)
        bad = details(0)
        bad[168+25] = 1  # Nonzero cycles with zero invocations.
        with self.assertRaises(ValueError): decode(bad)
        with self.assertRaises(ValueError): summarize([row,decode(packet(sequence=1))],"mixed")

    def test_sequence_wrap_and_large_counts(self):
        first = decode(packet(sequence=0xffffffff,total_cycles=0xffffffffffffffff))
        second = decode(packet(sequence=0))
        self.assertEqual(first["total_cycles"],0xffffffffffffffff)
        self.assertEqual(summarize([first,second],"test")["missed_reports"],0)


if __name__ == "__main__": unittest.main()
