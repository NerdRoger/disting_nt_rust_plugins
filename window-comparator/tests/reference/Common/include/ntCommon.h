#pragma once
#include "common.h"

#include <distingnt/api.h>


INLINE constexpr uint16_t CalculateScaling(int scale) {
	switch (scale)
	{
		case kNT_scaling10:   return 10;
		case kNT_scaling100:  return 100;
		case kNT_scaling1000: return 1000;
		default: return 1;
	}
}


// I use this version to set parameter values for consistency.  Some methods/functions won't know which is appropriate to
// call without a calling context passed along the call stack, because they can run from either thread.  Thus this function,
// used to put the choice of which NT_ version to call in a single place.

INLINE void SetParameterValue(uint32_t algorithmIndex, uint32_t parameter, int16_t value, CallingContext ctx) {
	if (ctx == CallingContext::AudioThread) {
		NT_setParameterFromAudio(algorithmIndex, parameter, value);
	} else {
		NT_setParameterFromUi(algorithmIndex, parameter, value);
	}
}


INLINE float GetScaledParameterValue(_NT_algorithm& alg, uint16_t paramIndex) {
	return static_cast<float>(alg.v[paramIndex]) / CalculateScaling(alg.parameters[paramIndex].scaling);
}


INLINE int16_t UnscaleValueForParameter(_NT_algorithm& alg, uint16_t paramIndex, float val) {
	return val * CalculateScaling(alg.parameters[paramIndex].scaling);
}



// #ifdef DEBUG
// 	__attribute__((always_inline)) 
// 	inline constexpr void LogMessage(const char* msg) {
// 		uint8_t midibuf[] = {
// 			0xF0, // sysex start
// 			0x7C, // non-commercial identifier:  https://midi.org/new-midi-association-sysex-id-policies-as-of-oct-15-2025
// 		};
				
// 		NT_sendMidiSysEx(kNT_destinationUSB, midibuf, sizeof(midibuf), false);
// 		NT_sendMidiSysEx(kNT_destinationUSB, reinterpret_cast<const uint8_t*>(msg), strlen(msg), true);
// 	}
// #else
// 	#define LogMessage(msg) ((void)0)
// #endif





// LLMs helped me write this implementation...  it takes multiple parameters and can print strings, ints, and floats
// LogMessage("Test message ", 123, " with float ", 45.67f);

#ifdef DEBUG
	inline char* _append_to_buf(char* ptr, char* end, const char* s) {
		while (s && *s && ptr < end) *ptr++ = *s++;
		return ptr;
	}

	inline char* _append_to_buf(char* ptr, char* end, int value) {
		if (value == 0) { 
			if (ptr < end) *ptr++ = '0'; 
			return ptr; 
		}
		if (value < 0) { 
			if (ptr < end) *ptr++ = '-'; 
			value = -value; 
		}
		char tmp[12];
		int i = 0;
		while (value > 0 && i < 12) { 
			tmp[i++] = (value % 10) + '0'; 
			value /= 10; 
		}
		while (i > 0 && ptr < end) { 
			*ptr++ = tmp[--i]; 
		}
		return ptr;
	}

	inline char* _append_to_buf(char* ptr, char* end, float value) {
		if (ptr >= end) return ptr;

		if (value < 0) {
			*ptr++ = '-';
			value = -value;
		}

		// Integer part
		int ipart = static_cast<int>(value);
		ptr = _append_to_buf(ptr, end, ipart);

		if (ptr < end) *ptr++ = '.';

		// Fractional part (5 digits)
		float fpart = value - static_cast<float>(ipart);
		int f_int = static_cast<int>(fpart * 100000.0f + 0.5f);

		// Manual leading zeros for the fraction (e.g. .00123)
		int divisor = 10000;
		while (divisor > 1 && f_int < divisor && ptr < end) {
			*ptr++ = '0';
			divisor /= 10;
		}

		return _append_to_buf(ptr, end, f_int);
	}

	inline void _RawLogSend(const char* buf, size_t len) {
		uint8_t header[] = { 0xF0, 0x7C };
		// Send header (0xF0 0x7C)
		NT_sendMidiSysEx(kNT_destinationUSB, header, sizeof(header), false);
		// Send message body and terminate with 0xF7 (last param = true)
		NT_sendMidiSysEx(kNT_destinationUSB, reinterpret_cast<const uint8_t*>(buf), len, true);
	}

	template<typename... Args>
	void LogMessage(Args... args) {
		char buf[128];
		char* ptr = buf;
		char* end = buf + 127;

		// Fold expression: calls the correct _append_to_buf for each argument
		((ptr = _append_to_buf(ptr, end, args)), ...);
		
		*ptr = '\0';
		_RawLogSend(buf, ptr - buf);
	}
#else
    // Release mode: entirely erased by preprocessor
    #define LogMessage(...) ((void)0)
#endif





void StringConcat(char* dest, int maxLen, ...);
