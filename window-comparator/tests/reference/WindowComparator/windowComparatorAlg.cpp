#include <distingnt/serialisation.h>
#include <cstring>
#include "ntCommon.h"
#include "windowComparatorAlg.h"


static constexpr float GateHigh = 10.0;
static constexpr float GateLow = 0.0;
static constexpr float TrigHigh = 10.0;
static constexpr float TrigLow = 0.0;


static const _NT_specification SpecificationsDef[] = {
	{ .name = "Channels", .min = 1, .max = 8, .def = 1, .type = kNT_typeGeneric },
};


static const char* const EnumStringsOverrideGlobalAtten[] = { "No", "Yes" };


static const char* const ChannelPageNames[] = {
	"Channel A", "Channel B", "Channel C", "Channel D", "Channel E", "Channel F", "Channel G", "Channel H",
};


static const char* const ExactlyInside[] = {
	"Exactly 1 Inside Gate", "Exactly 2 Inside Gate", "Exactly 3 Inside Gate", "Exactly 4 Inside Gate", "Exactly 5 Inside Gate", "Exactly 6 Inside Gate", "Exactly 7 Inside Gate",
};


static const char* const ExactlyOutside[] = {
	"Exactly 1 Outside Gate", "Exactly 2 Outside Gate", "Exactly 3 Outside Gate", "Exactly 4 Outside Gate", "Exactly 5 Outside Gate", "Exactly 6 Outside Gate", "Exactly 7 Outside Gate",
};


static const char* const AtLeastInside[] = {
	"At Least 1 Inside Gate", "At Least 2 Inside Gate", "At Least 3 Inside Gate", "At Least 4 Inside Gate", "At Least 5 Inside Gate", "At Least 6 Inside Gate", "At Least 7 Inside Gate",
};
	

static const char* const AtLeastOutside[] = {
	"At Least 1 Outside Gate", "At Least 2 Outside Gate", "At Least 3 Outside Gate", "At Least 4 Outside Gate", "At Least 5 Outside Gate", "At Least 6 Outside Gate", "At Least 7 Outside Gate",
};


uint8_t WindowComparatorAlg::CountParameters(uint8_t numChannels) {
	uint8_t result = 0 + kNumCommonParameters + kNumPerChannelParameters;
	if (numChannels > 1) {
		result += kNumCommonAggregateParameters + (numChannels - 1) * (0 + kNumPerChannelParameters + kNumPerChannelAggregateParameters);
	}
	return result;
}


void WindowComparatorAlg::InjectDependencies(const Dependencies& dependencies) {
	NumChannels = dependencies.NumChannels;
	TriggerSampleLength = dependencies.Globals->sampleRate / 1000;
	Timer.InjectDependencies({ .Globals = dependencies.Globals });
	ComparatorView::Dependencies viewDependencies;
	viewDependencies.Timer = &Timer;
	viewDependencies.Algorithm = this;
	View.InjectDependencies(viewDependencies);
}


void WindowComparatorAlg::BuildParameters() {

	// It may seem more natural at first glance for us to group all of the aggregate parameters together, at the end.
  // However, if we do that, then parameter numbers will change when we respecify the algorithm to use more or less channels
	// Laying them out in this manner ensures that as we add channels, all the new parameters go on the end, and
	// as we remove channels, the remaining parameters have the same ordinal index.
	// This way, every parameter is always at the same ordinal position, ergo respecify works as expected

	// Global
	ParameterDefs[kParamGlobalRangeMin] = { .name = "Range Min", .min = -10, .max = 10, .def = -5, .unit = kNT_unitConfirm, .scaling = kNT_scalingNone, .enumStrings = NULL };
	ParameterDefs[kParamGlobalRangeMax] = { .name = "Range Max", .min = -10, .max = 10, .def =  5, .unit = kNT_unitConfirm, .scaling = kNT_scalingNone, .enumStrings = NULL };

	ParameterDefs[kParamGlobalInputScale]  = { .name = "Atten Input",  .min = -20000, .max = 20000, .def = 10000, .unit = kNT_unitPercent, .scaling = kNT_scaling100, .enumStrings = NULL };
	ParameterDefs[kParamGlobalInputOffset] = { .name = "Offset Input", .min = -1000,  .max = 1000,  .def = 0,     .unit = kNT_unitVolts,   .scaling = kNT_scaling100, .enumStrings = NULL };
	
	// Comparator A
	int idx = kNumCommonParameters;
	ChannelOffsets[0] = idx;
	ParameterDefs[idx + kParamInput]               = { .name = "Input",                 .min = 0,      .max = 28,    .def = 1,     .unit = kNT_unitCvInput,  .scaling = kNT_scalingNone, .enumStrings = NULL };
	ParameterDefs[idx + kParamWindowLeft]          = { .name = "Window Left",           .min = -5000,  .max = 5000,  .def = -1000, .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamWindowRight]         = { .name = "Window Right",          .min = -5000,  .max = 5000,  .def = 1000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamWindowCenter]        = { .name = "Window Center",         .min = -5000,  .max = 5000,  .def = 0,     .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamWindowWidth]         = { .name = "Window Width",          .min = 0,      .max = 20000, .def = 2000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamOverrideGlobalAtten] = { .name = "Override Global Atten", .min = 0,      .max = 1,     .def = 0,     .unit = kNT_unitEnum,     .scaling = kNT_scalingNone, .enumStrings = EnumStringsOverrideGlobalAtten };
	ParameterDefs[idx + kParamInputScale]          = { .name = "Atten Input",           .min = -20000, .max = 20000, .def = 10000, .unit = kNT_unitPercent,  .scaling = kNT_scaling100,  .enumStrings = NULL };
	ParameterDefs[idx + kParamInputOffset]         = { .name = "Offset Input",          .min = -1000,  .max = 1000,  .def = 0,     .unit = kNT_unitVolts,    .scaling = kNT_scaling100,  .enumStrings = NULL };
	ParameterDefs[idx + kParamValueXContribution]  = { .name = "Value X Contribution",  .min = -10000, .max = 10000, .def = 1000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamValueYContribution]  = { .name = "Value Y Contribution",  .min = -10000, .max = 10000, .def = 500,   .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamValueZContribution]  = { .name = "Value Z Contribution",  .min = -10000, .max = 10000, .def = 100,   .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
	ParameterDefs[idx + kParamInsideWindowGate]    = { .name = "Inside Gate",           .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
	ParameterDefs[idx + kParamOutsideWindowGate]   = { .name = "Outside Gate",          .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
	ParameterDefs[idx + kParamEnterTrigger]        = { .name = "Enter Trig",            .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
	ParameterDefs[idx + kParamExitTrigger]         = { .name = "Exit Trig",             .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
	
	// Common Aggregate Parameters
	if (NumChannels > 1) {
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueXInsideTotal]    = { .name = "Value X Total Inside",  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueXOutsideTotal]   = { .name = "Value X Total Outside", .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueXInsideAverage]  = { .name = "Value X Avg Inside",    .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueXOutsideAverage] = { .name = "Value X Avg Outside",   .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueYInsideTotal]    = { .name = "Value Y Total Inside",  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueYOutsideTotal]   = { .name = "Value Y Total Outside", .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueYInsideAverage]  = { .name = "Value Y Avg Inside",    .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueYOutsideAverage] = { .name = "Value Y Avg Outside",   .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueZInsideTotal]    = { .name = "Value Z Total Inside",  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueZOutsideTotal]   = { .name = "Value Z Total Outside", .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueZInsideAverage]  = { .name = "Value Z Avg Inside",    .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamValueZOutsideAverage] = { .name = "Value Z Avg Outside",   .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamAllInsideWindowGate]  = { .name = "All Inside Gate",  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[0 + kNumCommonParameters + kNumPerChannelParameters + kParamAllOutsideWindowGate] = { .name = "All Outside Gate", .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
	}

	idx = 0 + kNumCommonParameters + kNumPerChannelParameters + kNumCommonAggregateParameters;
	for (uint8_t ch = 1; ch < NumChannels; ch++) {
		// Comparator
		ChannelOffsets[ch] = idx;
		ParameterDefs[idx + kParamInput]               = { .name = "Input",                 .min = 0,      .max = 28,    .def = 1,     .unit = kNT_unitCvInput,  .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[idx + kParamWindowLeft]          = { .name = "Window Left",           .min = -5000,  .max = 5000,  .def = -1000, .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamWindowRight]         = { .name = "Window Right",          .min = -5000,  .max = 5000,  .def = 1000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamWindowCenter]        = { .name = "Window Center",         .min = -5000,  .max = 5000,  .def = 0,     .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamWindowWidth]         = { .name = "Window Width",          .min = 0,      .max = 20000, .def = 2000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamOverrideGlobalAtten] = { .name = "Override Global Atten", .min = 0,      .max = 1,     .def = 0,     .unit = kNT_unitEnum,     .scaling = kNT_scalingNone, .enumStrings = EnumStringsOverrideGlobalAtten };
		ParameterDefs[idx + kParamInputScale]          = { .name = "Atten Input",           .min = -20000, .max = 20000, .def = 10000, .unit = kNT_unitPercent,  .scaling = kNT_scaling100,  .enumStrings = NULL };
		ParameterDefs[idx + kParamInputOffset]         = { .name = "Offset Input",          .min = -1000,  .max = 1000,  .def = 0,     .unit = kNT_unitVolts,    .scaling = kNT_scaling100,  .enumStrings = NULL };
		ParameterDefs[idx + kParamValueXContribution]  = { .name = "Value X Contribution",  .min = -10000, .max = 10000, .def = 1000,  .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamValueYContribution]  = { .name = "Value Y Contribution",  .min = -10000, .max = 10000, .def = 500,   .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamValueZContribution]  = { .name = "Value Z Contribution",  .min = -10000, .max = 10000, .def = 100,   .unit = kNT_unitVolts,    .scaling = kNT_scaling1000, .enumStrings = NULL };
		ParameterDefs[idx + kParamInsideWindowGate]    = { .name = "Inside Gate",           .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[idx + kParamOutsideWindowGate]   = { .name = "Outside Gate",          .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[idx + kParamEnterTrigger]        = { .name = "Enter Trig",            .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };
		ParameterDefs[idx + kParamExitTrigger]         = { .name = "Exit Trig",             .min = 0,      .max = 28,    .def = 0,     .unit = kNT_unitCvOutput, .scaling = kNT_scalingNone, .enumStrings = NULL };

		// Aggregate Parameters
		ParameterDefs[idx + kNumPerChannelParameters + kParamExactlyNInsideWindowGate]  = { .name = ExactlyInside[ch - 1],  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = 0, .enumStrings = NULL };
		ParameterDefs[idx + kNumPerChannelParameters + kParamExactlyNOutsideWindowGate] = { .name = ExactlyOutside[ch - 1], .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = 0, .enumStrings = NULL };
		ParameterDefs[idx + kNumPerChannelParameters + kParamAtLeastNInsideWindowGate]  = { .name = AtLeastInside[ch - 1],  .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = 0, .enumStrings = NULL };
		ParameterDefs[idx + kNumPerChannelParameters + kParamAtLeastNOutsideWindowGate] = { .name = AtLeastOutside[ch - 1], .min = 0, .max = 28, .def = 0, .unit = kNT_unitCvOutput, .scaling = 0, .enumStrings = NULL };
		idx += 0 + kNumPerChannelParameters + kNumPerChannelAggregateParameters;
	}
	parameters = ParameterDefs;
}


void WindowComparatorAlg::BuildParameterPages() {
	int numPages = 0;
	uint8_t* pagePtr = PageLayout;

	// global page
	pagePtr[0] = kParamGlobalRangeMin;
	pagePtr[1] = kParamGlobalRangeMax;
	pagePtr[2] = kParamGlobalInputScale;
	pagePtr[3] = kParamGlobalInputOffset;
	PageDefs[numPages] = { .name = "Global", .numParams = kNumCommonParameters, .group = 1, .params = pagePtr };
	pagePtr += kNumCommonParameters;
	numPages++;

	// channels
	for (uint8_t ch = 0; ch < NumChannels; ch++) {
		uint8_t offset = ChannelOffsets[ch];
		pagePtr[0]  = offset + kParamInput;
		pagePtr[1]  = offset + kParamWindowLeft;
		pagePtr[2]  = offset + kParamWindowRight;
		pagePtr[3]  = offset + kParamWindowCenter;
		pagePtr[4]  = offset + kParamWindowWidth;
		pagePtr[5]  = offset + kParamOverrideGlobalAtten;
		pagePtr[6]  = offset + kParamInputScale;
		pagePtr[7]  = offset + kParamInputOffset;
		pagePtr[8]  = offset + kParamValueXContribution;
		pagePtr[9]  = offset + kParamValueYContribution;
		pagePtr[10] = offset + kParamValueZContribution;
		pagePtr[11] = offset + kParamInsideWindowGate;
		pagePtr[12] = offset + kParamOutsideWindowGate;
		pagePtr[13] = offset + kParamEnterTrigger;
		pagePtr[14] = offset + kParamExitTrigger;
		PageDefs[numPages] = { .name = ChannelPageNames[ch], .numParams = kNumPerChannelParameters, .group = 2, .params = pagePtr };
		pagePtr += kNumPerChannelParameters;
		numPages++;
	}

	if (NumChannels > 1) {
		// aggregate gates page
		auto aggPagePtr = pagePtr;
		uint8_t cnt = 0;
		for (uint8_t ch = 1; ch < NumChannels; ch++) {
			uint8_t offset = 0 + kNumCommonParameters;
			offset += 0 + kNumPerChannelParameters + kNumCommonAggregateParameters;
			offset += (ch - 1) * (0 + kNumPerChannelParameters + kNumPerChannelAggregateParameters);
			offset += kNumPerChannelParameters;

			pagePtr[0] = offset + kParamExactlyNInsideWindowGate;
			pagePtr[1] = offset + kParamExactlyNOutsideWindowGate;
			pagePtr[2] = offset + kParamAtLeastNInsideWindowGate;
			pagePtr[3] = offset + kParamAtLeastNOutsideWindowGate;
			pagePtr += kNumPerChannelAggregateParameters;
			cnt += kNumPerChannelAggregateParameters;
		}

		int offset = 0 + kNumCommonParameters + kNumPerChannelParameters;
		pagePtr[0] = offset + kParamAllInsideWindowGate;
		pagePtr[1] = offset + kParamAllOutsideWindowGate;
		pagePtr += 2;
		cnt += 2;

		PageDefs[numPages] = { .name = "Agg. Gate", .numParams = cnt, .group = 3, .params = aggPagePtr };
		numPages++;

		// aggregate values page
		aggPagePtr = pagePtr;
		offset = 0 + kNumCommonParameters + kNumPerChannelParameters;
		pagePtr[0]  = offset + kParamValueXInsideTotal;
		pagePtr[1]  = offset + kParamValueXOutsideTotal;
		pagePtr[2]  = offset + kParamValueXInsideAverage;
		pagePtr[3]  = offset + kParamValueXOutsideAverage;
		pagePtr[4]  = offset + kParamValueYInsideTotal;
		pagePtr[5]  = offset + kParamValueYOutsideTotal;
		pagePtr[6]  = offset + kParamValueYInsideAverage;
		pagePtr[7]  = offset + kParamValueYOutsideAverage;
		pagePtr[8]  = offset + kParamValueZInsideTotal;
		pagePtr[9]  = offset + kParamValueZOutsideTotal;
		pagePtr[10] = offset + kParamValueZInsideAverage;
		pagePtr[11] = offset + kParamValueZOutsideAverage;
		pagePtr += 12;
		cnt = 12;
		PageDefs[numPages] = { .name = "Agg. Value", .numParams = cnt, .group = 4, .params = aggPagePtr };
		numPages++;

	}

	PagesDefs.numPages = numPages;
	PagesDefs.pages = PageDefs;

	parameterPages = &PagesDefs;
}


void WindowComparatorAlg::CalculateRequirements(_NT_algorithmRequirements& req, const int32_t* specifications) {
	uint8_t numChannels = specifications[0];
	req.numParameters = CountParameters(numChannels);
	req.sram = 0;
	req.dram = 0;
	req.dtc = 0;
	req.itc = 0;

	// calculate the memory requirements for all of the objects and dynamic arrays we need to create
	// THIS MUST STAY IN SYNC WITH THE CONSTRUCTION REQUIREMENTS IN Construct() BELOW
	MemoryHelper<WindowComparatorAlg>::AlignAndIncrementMemoryRequirement(req.sram, 1); // WindowComparatorAlg
	MemoryHelper<_NT_parameter>::AlignAndIncrementMemoryRequirement(req.sram, req.numParameters); // WindowComparatorAlg::ParameterDefs
	MemoryHelper<_NT_parameterPage>::AlignAndIncrementMemoryRequirement(req.sram, numChannels + 3); // WindowComparatorAlg::PageDefs
	MemoryHelper<uint8_t>::AlignAndIncrementMemoryRequirement(req.sram, req.numParameters); // WindowComparatorAlg::PageLayout
	MemoryHelper<uint8_t>::AlignAndIncrementMemoryRequirement(req.sram, numChannels); // WindowComparatorAlg::ChannelOffsets
	MemoryHelper<bool>::AlignAndIncrementMemoryRequirement(req.sram, numChannels); // WindowComparatorAlg::PreviouslyInside
	MemoryHelper<float>::AlignAndIncrementMemoryRequirement(req.sram, numChannels); // WindowComparatorAlg::CurrentValues
	MemoryHelper<bool>::AlignAndIncrementMemoryRequirement(req.sram, numChannels); // WindowComparatorAlg::UpdatingBounds
	MemoryHelper<bool>::AlignAndIncrementMemoryRequirement(req.sram, numChannels); // WindowComparatorAlg::UpdatingSizePos
}


_NT_algorithm* WindowComparatorAlg::Construct(const _NT_algorithmMemoryPtrs& ptrs, const _NT_algorithmRequirements& req, const int32_t* specifications) {
	uint8_t numChannels = specifications[0];
	auto mem = ptrs.sram;
	memset(mem, 0, req.sram);

	// initialize arrays that depend on number of channels
	// THIS MUST STAY IN SYNC WITH THE REQUIREMENTS OF CALCULATION IN CalculateRequirements() ABOVE
	auto& alg = *MemoryHelper<WindowComparatorAlg>::InitializeDynamicDataAndIncrementPointer(mem, 1);
	alg.ParameterDefs = MemoryHelper<_NT_parameter>::InitializeDynamicDataAndIncrementPointer(mem, req.numParameters);
	alg.PageDefs = MemoryHelper<_NT_parameterPage>::InitializeDynamicDataAndIncrementPointer(mem, numChannels + 3);
	alg.PageLayout = MemoryHelper<uint8_t>::InitializeDynamicDataAndIncrementPointer(mem, req.numParameters);
	alg.ChannelOffsets = MemoryHelper<uint8_t>::InitializeDynamicDataAndIncrementPointer(mem, numChannels);
	alg.PreviouslyInside = MemoryHelper<bool>::InitializeDynamicDataAndIncrementPointer(mem, numChannels);
	alg.CurrentValues = MemoryHelper<float>::InitializeDynamicDataAndIncrementPointer(mem, numChannels);
	alg.UpdatingBounds = MemoryHelper<bool>::InitializeDynamicDataAndIncrementPointer(mem, numChannels);
	alg.UpdatingSizePos = MemoryHelper<bool>::InitializeDynamicDataAndIncrementPointer(mem, numChannels);

	alg.InjectDependencies({ .NumChannels = static_cast<uint8_t>(numChannels), .Globals = &NT_globals });

	alg.BuildParameters();
	alg.BuildParameterPages();

	return &alg;
}


void WindowComparatorAlg::ParameterChanged(_NT_algorithm* self, int p) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	auto algIndex = NT_algorithmIndex(self);

	if (p == kParamGlobalRangeMin) {
		alg.RangeMin = GetScaledParameterValue(alg, kParamGlobalRangeMin);
		alg.ParameterDefs[kParamGlobalRangeMax].min = alg.v[kParamGlobalRangeMin];
		NT_updateParameterDefinition(algIndex, kParamGlobalRangeMax);
		alg.Range = alg.RangeMax - alg.RangeMin;
	}

	if (p == kParamGlobalRangeMax) {
		alg.RangeMax = GetScaledParameterValue(alg, kParamGlobalRangeMax);
		alg.ParameterDefs[kParamGlobalRangeMin].max = alg.v[kParamGlobalRangeMax];
		NT_updateParameterDefinition(algIndex, kParamGlobalRangeMin);
		alg.Range = alg.RangeMax - alg.RangeMin;
	}

	for (uint8_t ch = 0; ch < alg.NumChannels; ch++) {
		auto offset = alg.ChannelOffsets[ch];
		if (p == offset + kParamOverrideGlobalAtten) {
			bool override = alg.v[p] == 1;
			NT_setParameterGrayedOut(algIndex, offset + kParamInputScale  + NT_parameterOffset(), !override);
			NT_setParameterGrayedOut(algIndex, offset + kParamInputOffset + NT_parameterOffset(), !override);
		}

		if (p == offset + kParamWindowLeft) {
			alg.UpdatingBounds[ch] = true;
			auto winLeft   = alg.v[offset + kParamWindowLeft];
			auto winRight  = alg.v[offset + kParamWindowRight];
			auto winCenter = (winRight + winLeft) / 2;
			auto winWidth  = winRight - winLeft;
			if (!alg.UpdatingSizePos[ch]) {
				SetParameterValue(algIndex, offset + kParamWindowCenter + NT_parameterOffset(), winCenter, CallingContext::AudioThread);
				SetParameterValue(algIndex, offset + kParamWindowWidth  + NT_parameterOffset(), winWidth, CallingContext::AudioThread);
			}

			alg.ParameterDefs[offset + kParamWindowRight].min = alg.v[offset + kParamWindowLeft];
			NT_updateParameterDefinition(algIndex, offset + kParamWindowRight);
			alg.UpdatingBounds[ch] = false;
		}

		if (p == offset + kParamWindowRight) {
			alg.UpdatingBounds[ch] = true;
			auto winLeft   = alg.v[offset + kParamWindowLeft];
			auto winRight  = alg.v[offset + kParamWindowRight];
			auto winCenter = (winRight + winLeft) / 2;
			auto winWidth  = winRight - winLeft;
			if (!alg.UpdatingSizePos[ch]) {
				SetParameterValue(algIndex, offset + kParamWindowCenter + NT_parameterOffset(), winCenter, CallingContext::AudioThread);
				SetParameterValue(algIndex, offset + kParamWindowWidth  + NT_parameterOffset(), winWidth, CallingContext::AudioThread);
			}

			alg.ParameterDefs[offset + kParamWindowLeft].max = alg.v[offset + kParamWindowRight];
			NT_updateParameterDefinition(algIndex, offset + kParamWindowLeft);
			alg.UpdatingBounds[ch] = false;
		}

		if (p == offset + kParamWindowCenter || p == offset + kParamWindowWidth) {
			alg.UpdatingSizePos[ch] = true;
			auto winCenter = alg.v[offset + kParamWindowCenter];
			auto winWidth  = alg.v[offset + kParamWindowWidth];
			auto winLeft   = winCenter - winWidth / 2;
			auto winRight  = winCenter + winWidth / 2;
			if (!alg.UpdatingBounds[ch]) {
				SetParameterValue(algIndex, offset + kParamWindowLeft  + NT_parameterOffset(), winLeft, CallingContext::AudioThread);
				SetParameterValue(algIndex, offset + kParamWindowRight + NT_parameterOffset(), winRight, CallingContext::AudioThread);
			}
			alg.UpdatingSizePos[ch] = false;
		}

		if (p == kParamGlobalRangeMin || p == kParamGlobalRangeMax) {
			int8_t min = GetScaledParameterValue(alg, kParamGlobalRangeMin);
			int8_t max = GetScaledParameterValue(alg, kParamGlobalRangeMax);

			alg.ParameterDefs[offset + kParamWindowLeft].min = UnscaleValueForParameter(alg, offset + kParamWindowLeft, min);
			NT_updateParameterDefinition(algIndex, offset + kParamWindowLeft);

			alg.ParameterDefs[offset + kParamWindowRight].max = UnscaleValueForParameter(alg, offset + kParamWindowRight, max);
			NT_updateParameterDefinition(algIndex, offset + kParamWindowRight);
			
			alg.ParameterDefs[offset + kParamWindowCenter].min = UnscaleValueForParameter(alg, offset + kParamWindowCenter, min);
			alg.ParameterDefs[offset + kParamWindowCenter].max = UnscaleValueForParameter(alg, offset + kParamWindowCenter, max);
			NT_updateParameterDefinition(algIndex, offset + kParamWindowCenter);

			alg.ParameterDefs[offset + kParamWindowWidth].max = UnscaleValueForParameter(alg, offset + kParamWindowWidth, max - min);
			NT_updateParameterDefinition(algIndex, offset + kParamWindowWidth);
		}

	}
}


bool WindowComparatorAlg::Draw(_NT_algorithm* self) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	alg.View.Draw();
	return true;
}


uint32_t WindowComparatorAlg::HasCustomUI(_NT_algorithm* self) {
	return kNT_potL | kNT_potR | kNT_encoderL | kNT_encoderR | kNT_encoderButtonR | kNT_potButtonR;
}


void WindowComparatorAlg::CustomUI(_NT_algorithm* self, const _NT_uiData& data) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	alg.View.ProcessControlInput(data);
	alg.PotMgr.RecordPreviousPotValues(data);
}


void WindowComparatorAlg::SetupUI(_NT_algorithm* self, _NT_float3& pots) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	alg.View.FixupPotValues(pots);
}


int WindowComparatorAlg::ParameterUiPrefix(_NT_algorithm* self, int p, char* buff) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	for (uint8_t ch = 0; ch < alg.NumChannels; ch++) {
		auto offset = alg.ChannelOffsets[ch];
		if (p >= offset && p < offset + kNumPerChannelParameters) {
			buff[0] = 'A' + ch;
			buff[1] = ':';
			buff[2] = 0;
			// return the number of chars to render, i.e. length NOT counting the null terminator
			return 2;
		}
	}
	return 0;
}


void WindowComparatorAlg::Step(_NT_algorithm* self, float* busFrames, int numFramesBy4) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);
	auto numFrames = numFramesBy4 * 4;

	// we only want to clear the block for the outputs before writing to them
	// after that, we want our outputs to "stack".  This will track, per bus, if we 
	// have already cleared the block.
	bool clearedThisStep[MAX_BUS_COUNT] = { };

	// a lambda to clear the block if necessary
	auto clearBlockIfRequired = [&](uint8_t busIndex) {
		if (!clearedThisStep[busIndex]) {
			memset(busFrames + (busIndex * numFrames), 0, numFrames * sizeof(float));
			clearedThisStep[busIndex] = true;
		}
	};

	// a lambda to write an event block to a bus
	auto writeEventToBusBlock = [&](int8_t busIndex, bool high, uint16_t* remainingSamplesPtr = nullptr) {
		if (busIndex < 0)
			return;

		clearBlockIfRequired(busIndex);
		float* busBuffer = busFrames + (busIndex * numFrames);

		if (high && remainingSamplesPtr) {
			remainingSamplesPtr[busIndex] = alg.TriggerSampleLength;
		}

		// only pull the buffer up, never down
		// multiple events could be writing triggers/gates into our buffer
		// and we want them to "stack"
		if (high) {
			// fill the whole output block with our result
			for (int i = 0; i < numFrames; i++) {
				busBuffer[i] = GateHigh;
			}
		}

		if (remainingSamplesPtr) {
			remainingSamplesPtr[busIndex] = remainingSamplesPtr[busIndex] <= numFrames ? 0 : remainingSamplesPtr[busIndex] - numFrames;
		}
	};

	// a lambda to write a value block to a bus
	auto writeValueToBusBlock = [&](int8_t busIndex, float val) {
		if (busIndex < 0)
			return;

		float* busBuffer = busFrames + (busIndex * numFrames);
		for (int i = 0; i < numFrames; i++) {
			busBuffer[i] = val;
		}
	};

	uint8_t insideCount = 0;
	uint8_t outsideCount = 0;
	float insideXTotal = 0;
	float outsideXTotal = 0;
	float insideYTotal = 0;
	float outsideYTotal = 0;
	float insideZTotal = 0;
	float outsideZTotal = 0;

	for (uint8_t ch = 0; ch < alg.NumChannels; ch++) {
		auto offset = alg.ChannelOffsets[ch];
		auto inputBusIndex = alg.v[offset + kParamInput] - 1;

		// get the bounds for the window
		auto winLeft  = GetScaledParameterValue(alg, offset + kParamWindowLeft);
		auto winRight = GetScaledParameterValue(alg, offset + kParamWindowRight);

		// get the value
		bool override = alg.v[offset + kParamOverrideGlobalAtten] == 1;
		auto valScale  = override ? GetScaledParameterValue(alg, offset + kParamInputScale)  : GetScaledParameterValue(alg, kParamGlobalInputScale);
		auto valOffset = override ? GetScaledParameterValue(alg, offset + kParamInputOffset) : GetScaledParameterValue(alg, kParamGlobalInputOffset);
		auto val = (inputBusIndex < 0) ? 0.0f : busFrames[inputBusIndex * numFrames];
		val = val * valScale / 100.0f + valOffset;
		val = clamp(val, static_cast<float>(alg.RangeMin), static_cast<float>(alg.RangeMax));
		alg.CurrentValues[ch] = val;

		// calculate our state(s)
		bool inside = val >= winLeft && val <= winRight;
		// don't set entered/exited on the very first step, to prevent triggers from firing when loading a preset
		bool entered = inside && !alg.PreviouslyInside[ch] && !alg.FirstStep;
		bool exited = !inside && alg.PreviouslyInside[ch] && !alg.FirstStep;
		alg.PreviouslyInside[ch] = inside;
		if (inside) {
			insideCount++;
			insideXTotal += GetScaledParameterValue(alg, offset + kParamValueXContribution);
			insideYTotal += GetScaledParameterValue(alg, offset + kParamValueYContribution);
			insideZTotal += GetScaledParameterValue(alg, offset + kParamValueZContribution);
		} else {
			outsideCount++;
			outsideXTotal += GetScaledParameterValue(alg, offset + kParamValueXContribution);
			outsideYTotal += GetScaledParameterValue(alg, offset + kParamValueYContribution);
			outsideZTotal += GetScaledParameterValue(alg, offset + kParamValueZContribution);
		}

		// write the triggers/gates
		writeEventToBusBlock(alg.v[offset + kParamInsideWindowGate]  - 1, inside);
		writeEventToBusBlock(alg.v[offset + kParamOutsideWindowGate] - 1, !inside);
		writeEventToBusBlock(alg.v[offset + kParamEnterTrigger] - 1, entered, alg.TriggerRemainingSamples);
		writeEventToBusBlock(alg.v[offset + kParamExitTrigger]  - 1, exited,  alg.TriggerRemainingSamples);
	}

	// common aggregate parameters
	if (alg.NumChannels > 1) {
		auto offset = 0 + kNumCommonParameters + kNumPerChannelParameters;
  	writeEventToBusBlock(alg.v[offset + kParamAllInsideWindowGate]  - 1, insideCount  == alg.NumChannels);
  	writeEventToBusBlock(alg.v[offset + kParamAllOutsideWindowGate] - 1, outsideCount == alg.NumChannels);

		writeValueToBusBlock(alg.v[offset + kParamValueXInsideTotal]    - 1, insideXTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueXOutsideTotal]   - 1, outsideXTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueXInsideAverage]  - 1, insideXTotal / static_cast<float>(insideCount));
		writeValueToBusBlock(alg.v[offset + kParamValueXOutsideAverage] - 1, outsideXTotal / static_cast<float>(outsideCount));
		writeValueToBusBlock(alg.v[offset + kParamValueYInsideTotal]    - 1, insideYTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueYOutsideTotal]   - 1, outsideYTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueYInsideAverage]  - 1, insideYTotal / static_cast<float>(insideCount));
		writeValueToBusBlock(alg.v[offset + kParamValueYOutsideAverage] - 1, outsideYTotal / static_cast<float>(outsideCount));
		writeValueToBusBlock(alg.v[offset + kParamValueZInsideTotal]    - 1, insideZTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueZOutsideTotal]   - 1, outsideZTotal);
		writeValueToBusBlock(alg.v[offset + kParamValueZInsideAverage]  - 1, insideZTotal / static_cast<float>(insideCount));
		writeValueToBusBlock(alg.v[offset + kParamValueZOutsideAverage] - 1, outsideZTotal / static_cast<float>(outsideCount));
	}

	// per-channel aggregate parameters
	for (uint8_t ch = 1; ch < alg.NumChannels; ch++) {
		auto offset = alg.ChannelOffsets[ch] + kNumPerChannelParameters;
  	writeEventToBusBlock(alg.v[offset + kParamExactlyNInsideWindowGate]  - 1, insideCount == ch);
  	writeEventToBusBlock(alg.v[offset + kParamExactlyNOutsideWindowGate] - 1, outsideCount == ch);
  	writeEventToBusBlock(alg.v[offset + kParamAtLeastNInsideWindowGate]  - 1, insideCount >= ch);
  	writeEventToBusBlock(alg.v[offset + kParamAtLeastNOutsideWindowGate] - 1, outsideCount >= ch);
	}

	// we can do this tracking outside of the processing loop, because we don't need sample-level accuracy
	// we are only tracking milliseconds, so block-level accuracy should be fine
	alg.Timer.CountMilliseconds(numFrames);

	alg.FirstStep = false;
}


int WindowComparatorAlg::ParameterString(_NT_algorithm* self, int p, int v, char* buff)
{
	memset(buff, 0, kNT_parameterStringSize);
	if (p == kParamGlobalRangeMin || p == kParamGlobalRangeMax) {
		auto idx = NT_intToString(buff, v);
		buff[idx] = 'V';
		buff[idx + 1] = '\0';
	}
	return strlen(buff);
}


void WindowComparatorAlg::Serialise(_NT_algorithm* self, _NT_jsonStream& stream) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);

	stream.addMemberName("BoundsEditMode");
	stream.addBoolean(alg.View.BoundsEditMode);

	stream.addMemberName("Editable");
	stream.addBoolean(alg.View.Editable);
}


bool WindowComparatorAlg::Deserialise(_NT_algorithm* self, _NT_jsonParse& parse) {
	auto& alg = *static_cast<WindowComparatorAlg*>(self);

	int num;
	if (!parse.numberOfObjectMembers(num)) {
		return false;
	}

	for (int i = 0; i < num; i++) {
		if (parse.matchName("BoundsEditMode")) {
			bool val;
			if (!parse.boolean(val)) {
				return false;
			}
			alg.View.BoundsEditMode = val;
		} else if (parse.matchName("Editable")) {
			bool val;
			if (!parse.boolean(val)) {
				return false;
			}
			alg.View.Editable = val;
		} else {
			if (!parse.skipMember()) {
				return false;
			}
		}
	}

	return true;
}


const _NT_factory WindowComparatorAlg::Factory =
{
	.guid = NT_MULTICHAR( 'A', 'T', 'w', 'c' ),
	.name = "Window Comparator",
	.description = "A window comparator",
	.numSpecifications = ARRAY_SIZE(SpecificationsDef),
	.specifications = SpecificationsDef,
	.calculateRequirements = CalculateRequirements,
	.construct = Construct,
	.parameterChanged = ParameterChanged,
	.step = Step,
	.draw = Draw,
	.tags = kNT_tagUtility,
	.hasCustomUi = HasCustomUI,
	.customUi = CustomUI,
	.setupUi = SetupUI,
	.serialise = Serialise,
	.deserialise = Deserialise,
	.parameterUiPrefix = ParameterUiPrefix,
	.parameterString = ParameterString,
};
