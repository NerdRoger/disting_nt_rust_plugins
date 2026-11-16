import unittest
import check_api as api


class SignatureParsing(unittest.TestCase):
    def test_worker_callback_commas_do_not_split_outer_arguments(self):
        self.assertEqual(api.argument_types("void (*cb)( _NT_algorithm* alg, uint32_t data ), uint32_t index, uint32_t data"),
                         ['Option<unsafe extern "C" fn(*mut NtAlgorithm, u32)>', "u32", "u32"])

    def test_large_sysex_output_reference_to_const_bytes_is_mutable_pointer(self):
        self.assertEqual(api.argument_types("void* state, const uint8_t*& data, uint32_t& count, bool& end"),
                         ["*mut c_void", "*mut *const u8", "*mut u32", "*mut bool"])

    def test_const_object_reference_is_not_writable(self):
        self.assertEqual(api.rust_type("const _NT_algorithmRequirements&"), "*const NtAlgorithmRequirements")

    def test_fft_pointer_alias_and_const_spelling(self):
        self.assertEqual(api.rust_type("_NT_FFT"), "NtFft")
        self.assertEqual(api.rust_type("uint8_t const *"), "*const u8")
