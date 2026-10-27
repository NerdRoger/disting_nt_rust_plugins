"""Placement policy tests and an ARM ELF parser/relocation integration test."""
import unittest
import shutil
import subprocess
import tempfile
from pathlib import Path
from dram_callees import exclusive_callees, inspect, place


class PlacementTests(unittest.TestCase):
    def select(self, edges):
        return exclusive_callees({"dram", "a", "b", "itc", "unused"}, {"dram"}, edges)

    def test_recursive_callee_chain(self):
        self.assertEqual(self.select([("dram", "a", True), ("a", "b", True)]), {"a", "b"})

    def test_shared_callee_keeps_its_descendants(self):
        self.assertEqual(self.select([("dram", "a", True), ("itc", "a", True), ("a", "b", True)]), set())

    def test_unshared_parent_can_move_while_shared_child_stays(self):
        self.assertEqual(self.select([("dram", "a", True), ("a", "b", True), ("itc", "b", True)]), {"a"})

    def test_address_taken_callee_is_conservatively_retained(self):
        for owner in ["factory", "dram", "itc"]:
            self.assertEqual(self.select([("dram", "a", True), (owner, "a", False)]), set())

    def test_mutually_recursive_group(self):
        edges = [("dram", "a", True), ("a", "b", True), ("b", "a", True)]
        self.assertEqual(self.select(edges), {"a", "b"})
        self.assertEqual(self.select(edges + [("itc", "b", True)]), set())

    def test_no_roots_no_moves(self):
        self.assertEqual(exclusive_callees({"a", "b"}, set(), [("a", "b", True)]), set())

    @unittest.skipUnless(shutil.which("arm-none-eabi-gcc"), "requires GNU ARM tools")
    def test_real_arm_object_preserves_shared_and_address_taken_functions(self):
        with tempfile.TemporaryDirectory(prefix="nt-dram-test-") as directory:
            source, obj = Path(directory)/"calls.S", Path(directory)/"calls.o"
            assembly = ".syntax unified\n.thumb\n"
            functions = {
                "root": ["only", "shared", "address_taken"],
                "only": ["leaf"], "leaf": [], "shared": [],
                "address_taken": [], "itc": ["shared"],
            }
            for name, calls in functions.items():
                section = ".text.nt_dram" if name == "root" else ".text." + name
                assembly += f'.section {section},"ax",%progbits\n.global {name}\n.type {name},%function\n.thumb_func\n{name}:\npush {{r4,lr}}\n'
                assembly += "".join("bl " + callee + "\n" for callee in calls)
                assembly += "pop {r4,pc}\n"
            assembly += '.section .rodata.factory,"a",%progbits\n.word address_taken\n.section .note.GNU-stack,"",%progbits\n'
            source.write_text(assembly)
            subprocess.run(["arm-none-eabi-gcc", "-mcpu=cortex-m7", "-mthumb", "-c", str(source), "-o", str(obj)], check=True)
            sections, selected = inspect(obj, "arm-none-eabi-")
            self.assertEqual({sections[i][0] for i in selected}, {".text.only", ".text.leaf"})
            place(obj, "arm-none-eabi-")
            symbols = subprocess.check_output(["arm-none-eabi-objdump", "-t", str(obj)], text=True)
            for name in ["only", "leaf"]:
                self.assertRegex(symbols, r"(?m)^.*F \.text\.nt_dram\s+\w+ " + name + r"$")
            for name in ["shared", "address_taken", "itc"]:
                self.assertRegex(symbols, r"(?m)^.*F \.text\." + name + r"\s+\w+ " + name + r"$")


if __name__ == "__main__":
    unittest.main()
