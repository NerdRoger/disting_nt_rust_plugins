#pragma once
#include "common.h"
#include "comparatorView.h"
#include "potManager.h"
#include "helpTextHelper.h"


enum {
	kParamGlobalRangeMin,
	kParamGlobalRangeMax,
	kParamGlobalInputScale,
	kParamGlobalInputOffset,

	kNumCommonParameters,
};


enum {
	kParamInput,
	kParamWindowLeft,
	kParamWindowRight,
	kParamWindowCenter,
	kParamWindowWidth,
	kParamOverrideGlobalAtten,
	kParamInputScale,
	kParamInputOffset,
	kParamValueXContribution,
	kParamValueYContribution,
	kParamValueZContribution,
	kParamInsideWindowGate,
	kParamOutsideWindowGate,
	kParamEnterTrigger,
	kParamExitTrigger,

	kNumPerChannelParameters,
};


enum {
	kParamValueXInsideTotal,
	kParamValueXOutsideTotal,
	kParamValueXInsideAverage,
	kParamValueXOutsideAverage,
	kParamValueYInsideTotal,
	kParamValueYOutsideTotal,
	kParamValueYInsideAverage,
	kParamValueYOutsideAverage,
	kParamValueZInsideTotal,
	kParamValueZOutsideTotal,
	kParamValueZInsideAverage,
	kParamValueZOutsideAverage,
	kParamAllInsideWindowGate,
	kParamAllOutsideWindowGate,

	kNumCommonAggregateParameters,
};


enum {
	kParamExactlyNInsideWindowGate,
	kParamExactlyNOutsideWindowGate,
	kParamAtLeastNInsideWindowGate,
	kParamAtLeastNOutsideWindowGate,

	kNumPerChannelAggregateParameters,
};


struct WindowComparatorAlg : public _NT_algorithm {
public:
	struct Dependencies {
		uint8_t NumChannels = 0;
		const _NT_globals* Globals = nullptr;
	};

private:

	friend struct ComparatorView;

	_NT_parameter* ParameterDefs;
	_NT_parameterPages PagesDefs;
	_NT_parameterPage* PageDefs;
	uint8_t* PageLayout;
	uint8_t* ChannelOffsets;
	bool* PreviouslyInside;
	float* CurrentValues;
	bool* UpdatingBounds;
	bool* UpdatingSizePos;
	uint16_t TriggerRemainingSamples[MAX_BUS_COUNT] = { };
	uint16_t TriggerSampleLength;
	bool FirstStep = true;
	static uint8_t CountParameters(uint8_t numChannels);
	void BuildParameters();
	void BuildParameterPages();
	void InjectDependencies(const Dependencies& dependencies);

	// NT factory "methods"
	static void CalculateRequirements(_NT_algorithmRequirements& req, const int32_t* specifications);
	static _NT_algorithm* Construct(const _NT_algorithmMemoryPtrs& ptrs, const _NT_algorithmRequirements& req, const int32_t* specifications);
	static void ParameterChanged(_NT_algorithm* self, int p);
	static void Step(_NT_algorithm* self, float* busFrames, int numFramesBy4);
	static bool Draw(_NT_algorithm* self);
	static uint32_t HasCustomUI(_NT_algorithm* self);
	static void CustomUI(_NT_algorithm* self, const _NT_uiData& data);
	static void Serialise(_NT_algorithm* self, _NT_jsonStream& stream);
	static bool Deserialise(_NT_algorithm* self, _NT_jsonParse& parse);
	static void SetupUI(_NT_algorithm* self, _NT_float3& pots);
	static int ParameterUiPrefix(_NT_algorithm* self, int p, char* buff);
  static int ParameterString(_NT_algorithm* self, int p, int v, char* buff);

public:
	int8_t RangeMin = -5;
	int8_t RangeMax = 5;
	uint8_t Range = RangeMax - RangeMin;

	HIDDEN static const _NT_factory Factory;

	uint8_t NumChannels;

	TimeKeeper Timer;
	PotManager PotMgr;
	HelpTextHelper HelpText;
	ComparatorView View;

};