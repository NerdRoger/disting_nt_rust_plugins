#pragma once

#include <new>
#include <stdint.h>
#include <stdlib.h>


#ifndef HIDDEN
	#define HIDDEN __attribute__((visibility("hidden")))
#endif

#ifndef INLINE
	#define INLINE __attribute__((always_inline)) inline
#endif


constexpr uint8_t MAX_BUS_COUNT = 28;


enum class CallingContext {
	AudioThread,
	UiThread
};


struct Point {
	uint8_t x;
	uint8_t y;
};


struct Bounds {
	uint8_t x1;
	uint8_t y1;
	uint8_t x2;
	uint8_t y2;
};


template <typename T>
INLINE constexpr T min(T a, T b) {
	return (a < b) ? a : b;
}


template <typename T>
INLINE constexpr T max(T a, T b) {
	return (a > b) ? a : b;
}


template <typename T>
INLINE constexpr T clamp(T val, T lo, T hi) {
	return (val < lo) ? lo : (val > hi) ? hi : val;
}


template <typename T>
INLINE constexpr T wrap(T val, T lo, T hi) {
	const T range = hi - lo + 1;
	val = (val - lo) % range;
	if (val < 0) val += range;
	return val + lo;
}


template <typename T>
INLINE constexpr void lohi(T& a, T& b) {
	if (b < a) {
		T tmp = a;
		a = b;
		b = tmp;
	}
}


// TODO:  possibly track when the last transition was, to measure gate duration
struct Trigger {
private:
	static constexpr float LowThreshold = 0.1f;
	static constexpr float HighThreshold = 1.0f;

	bool State;
public:
	enum Edge {
		None,
		Rising,
		Falling
	};

	INLINE Edge Process(float val) {
		if (State && (val < LowThreshold)) {
			State = false;
			return Falling;
		} else if (!State && (val >= HighThreshold)) {
			State = true;
			return Rising;
		}
		return None;
	}
};



struct RandomGenerator {
private:
	uint32_t PrevRandom;

public:

	INLINE void Seed(uint32_t seed) {
		PrevRandom = seed;
	}

	INLINE uint32_t Next(uint32_t lowInclusive, uint32_t highInclusive) {
		uint32_t x = PrevRandom;
		x ^= x << 13;
		x ^= x >> 17;
		x ^= x << 5;
		PrevRandom = x;

		uint32_t diff = highInclusive - lowInclusive;
		x %= (diff + 1);
		x += lowInclusive;
		return x;
	}
};


// helper used when dynamically allocating/aligning memory for various types from the same memory block
template <typename T>
struct MemoryHelper {
private:
	static constexpr size_t size = sizeof(T);
	static constexpr size_t alignment = alignof(T);

public:
	static void AlignAndIncrementMemoryRequirement(uint32_t& memRequired, size_t numElements) {
		// align the memory requirement to the next alignment boundary for the type
		memRequired = (memRequired + alignment - 1) & ~(alignment - 1);
		// then increment it by the size of the type * number of elements
		memRequired += size * numElements;
	}

	static T* AlignMemoryPointer(uint8_t*& ptr) {
		uintptr_t addr = reinterpret_cast<uintptr_t>(ptr);
		// align the memory address to the next alignment boundary for the type
		addr = (addr + alignment - 1) & ~(alignment - 1);
		ptr = reinterpret_cast<uint8_t*>(addr);
		return reinterpret_cast<T*>(ptr);
	}

	// use this version with a factory function to construct each element
	template <typename F>
	static T* InitializeDynamicDataAndIncrementPointer(uint8_t*& ptr, size_t num, F factory) {
		// align the memory pointer to the boundary for the type
		T* result = AlignMemoryPointer(ptr);

		// let the supplied factory construct each element in place
		for (size_t i = 0; i < num; i++) {
			factory(result + i, i);
		}

		// advance the raw pointer by the total size of the allocated elements
		ptr = reinterpret_cast<uint8_t*>(result + num);

		return result;
	}

	// use this version if default construction is sufficient
	static T* InitializeDynamicDataAndIncrementPointer(uint8_t*& ptr, size_t num) {
		return InitializeDynamicDataAndIncrementPointer(ptr, num, [](T* addr, size_t) {
			new (addr) T();
		});
	}

};
