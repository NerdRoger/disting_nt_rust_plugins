import unittest
from monitor_cycles import decode, summarize, IDENTIFIER, FIELDS, WIDTHS


def packet(**changes):
    values = dict(instance=0,sequence=0,display_cycles=12000,total_cycles=24000000,callbacks=2000,
                  frames=48000,sample_rate=48000,block_frames=24,voices=10,midi=0,stereo=1)
    values.update(changes)
    data = list(IDENTIFIER + (1,))
    for key,width in zip(FIELDS,WIDTHS):
        data.extend((values[key] >> (7*i)) & 127 for i in range(width))
    return data


class MonitorTests(unittest.TestCase):
    def test_decode_roundtrip_and_weighted_mean(self):
        first = decode(packet(total_cycles=25000000))
        second = decode(packet(sequence=1,total_cycles=12000000,callbacks=1000))
        self.assertEqual(first["cycles_per_callback"],12500)
        result = summarize([first,second],"test")
        self.assertAlmostEqual(result["mean_cycles_per_callback"],37000000/3000)
        self.assertEqual(result["missed_reports"],0)

    def test_reject_malformed_and_changed_workload(self):
        self.assertIsNone(decode([0,33,39]))
        with self.assertRaises(ValueError): decode(packet()[:-1])
        with self.assertRaises(ValueError): decode(packet(callbacks=0))
        with self.assertRaises(ValueError): summarize([decode(packet()),decode(packet(sequence=1,voices=9))],"test")
        with self.assertRaises(ValueError): summarize([decode(packet()),decode(packet())],"test")

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

    def test_sequence_wrap_and_large_counts(self):
        first = decode(packet(sequence=0xffffffff,total_cycles=0xffffffffffffffff))
        second = decode(packet(sequence=0))
        self.assertEqual(first["total_cycles"],0xffffffffffffffff)
        self.assertEqual(summarize([first,second],"test")["missed_reports"],0)


if __name__ == "__main__": unittest.main()
