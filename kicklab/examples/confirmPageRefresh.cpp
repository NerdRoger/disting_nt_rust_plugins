// To reproduce, leave Main > Global at 1. On Model Test, choose Two and
// confirm it. The page shows "Two Value", but Model offers One for
// confirmation even though Two was committed. Changing Global to 3
// makes the spurious preview offer Three on the next page refresh.

#include <new>
#include <distingnt/api.h>

enum
{
	kParamGlobal,
	kParamModel,
	kParamValue,
	kParamExtraA,
	kParamExtraB,
};

static const _NT_parameter parameterDefs[] = {
	{ .name = "Global", .min = 0, .max = 3, .def = 1, .unit = kNT_unitNone, .scaling = 0, .enumStrings = nullptr },
	{ .name = "Model", .min = 0, .max = 3, .def = 0, .unit = kNT_unitConfirm, .scaling = 0, .enumStrings = nullptr },
	{ .name = "Value", .min = 0, .max = 10, .def = 0, .unit = kNT_unitNone, .scaling = 0, .enumStrings = nullptr },
	{ .name = "Extra A", .min = 0, .max = 10, .def = 0, .unit = kNT_unitNone, .scaling = 0, .enumStrings = nullptr },
	{ .name = "Extra B", .min = 0, .max = 10, .def = 0, .unit = kNT_unitNone, .scaling = 0, .enumStrings = nullptr },
};

static const uint8_t mainIndices[] = { kParamGlobal };
static const uint8_t pageIndices[] = { kParamModel, kParamValue, kParamExtraA, kParamExtraB };
static const char* const modelNames[] = { "Off", "One", "Two", "Three" };
static const char* const valueNames[] = { "Value", "One Value", "Two Value", "Three Value" };

struct _confirmPageRefresh : public _NT_algorithm
{
	_NT_parameter params[ARRAY_SIZE(parameterDefs)];
	_NT_parameterPage page[2];
	_NT_parameterPages pages;
	int model;
};

static void calculateRequirements( _NT_algorithmRequirements& req, const int32_t* )
{
	req.numParameters = ARRAY_SIZE(parameterDefs);
	req.sram = sizeof(_confirmPageRefresh);
	req.dram = 0;
	req.dtc = 0;
	req.itc = 0;
}

static _NT_algorithm* construct( const _NT_algorithmMemoryPtrs& ptrs, const _NT_algorithmRequirements&, const int32_t* )
{
	auto* alg = new (ptrs.sram) _confirmPageRefresh();
	for ( unsigned i = 0; i < ARRAY_SIZE(parameterDefs); ++i )
		alg->params[i] = parameterDefs[i];
	alg->page[0] = { .name = "Main", .numParams = ARRAY_SIZE(mainIndices), .params = mainIndices };
	alg->page[1] = { .name = "Model Test", .numParams = 1, .group = 1, .params = pageIndices };
	alg->pages = { .numPages = ARRAY_SIZE(alg->page), .pages = alg->page };
	alg->parameters = alg->params;
	alg->parameterPages = &alg->pages;
	alg->model = 0;
	return alg;
}

static void parameterChanged( _NT_algorithm* self, int parameter )
{
	if ( parameter != kParamModel )
		return;

	auto* alg = static_cast<_confirmPageRefresh*>(self);
	const int model = alg->v[kParamModel];
	if ( model < 0 || model >= static_cast<int>(ARRAY_SIZE(modelNames)) || model == alg->model )
		return;

	alg->model = model;
	const int algorithmIndex = NT_algorithmIndex(self);
	alg->params[kParamValue].name = valueNames[model];
	alg->params[kParamValue].max = model * 10;
	NT_updateParameterDefinition( algorithmIndex, kParamValue );

	alg->page[1].numParams = model + 1;
	NT_updateParameterPages( algorithmIndex );
}

static int parameterString( _NT_algorithm*, int parameter, int value, char* buff )
{
	if ( parameter != kParamModel || value < 0 || value >= static_cast<int>(ARRAY_SIZE(modelNames)) )
		return 0;
	const char* name = modelNames[value];
	int length = 0;
	while ( name[length] )
	{
		buff[length] = name[length];
		++length;
	}
	buff[length] = '\0';
	return length;
}

static const _NT_factory factory =
{
	.guid = NT_MULTICHAR( 'C', 'P', 'R', 'f' ),
	.name = "Confirm Page Refresh",
	.description = "Refreshes a parameter page after a confirmed Model change",
	.numSpecifications = 0,
	.calculateRequirements = calculateRequirements,
	.construct = construct,
	.parameterChanged = parameterChanged,
	.tags = kNT_tagUtility,
	.parameterString = parameterString,
};

uintptr_t pluginEntry( _NT_selector selector, uint32_t data )
{
	switch ( selector )
	{
	case kNT_selector_version:
		return kNT_apiVersionCurrent;
	case kNT_selector_numFactories:
		return 1;
	case kNT_selector_factoryInfo:
		return (uintptr_t)( ( data == 0 ) ? &factory : nullptr );
	}
	return 0;
}
