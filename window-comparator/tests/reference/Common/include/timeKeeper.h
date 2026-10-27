#pragma once
#include <stdint.h>


struct TimeKeeper {
	public:
		struct Dependencies {
			const _NT_globals* Globals = nullptr;
		};

	private:
		uint32_t InternalFrameCount;
		uint32_t SamplesPerMs;
	
	public:
		uint32_t TotalMs;

		TimeKeeper();
		void InjectDependencies(const Dependencies& dependencies);
		uint32_t CountMilliseconds(int numFrames);
	};
	
	