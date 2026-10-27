#include <string.h>
#include "comparatorView.h"
#include "windowComparatorAlg.h"
#include "ntCommon.h"


static constexpr uint8_t MaxDisplayedLines = 4;
static constexpr uint8_t LinePadding = 2;
static constexpr uint8_t BarHeight = 10;
static constexpr uint8_t BarWidth = 240;
static constexpr uint8_t BarLeft = 15;
static constexpr uint8_t SelectedBarColor = 6;
static constexpr uint8_t UnselectedBarColor = 2;
static constexpr uint8_t SelectedColor = 15;
static constexpr uint8_t UnselectedColor = 4;


static const char* const ChannelLabels[] = {
	"A", "B", "C", "D", "E", "F", "G", "H",
};


void ComparatorView::OnDrawHandler(ViewBase* view) {
	auto& cv = *static_cast<ComparatorView*>(view);
	cv.DrawComparators();
	cv.DrawHelpSection();
}


void ComparatorView::OnEncoder1TurnHandler(ViewBase* view, int8_t x) {
	auto& cv = *static_cast<ComparatorView*>(view);
	cv.SelectedComparatorIndex = clamp(cv.SelectedComparatorIndex + x, 0, cv.Algorithm->NumChannels - 1);
	cv.DisplayBarStatsHelpText();
}


void ComparatorView::OnEncoder2ShortPressHandler(ViewBase* view) {
	auto& cv = *static_cast<ComparatorView*>(view);
	cv.Editable = !cv.Editable;
}


void ComparatorView::OnEncoder2LongPressHandler(ViewBase* view) {
	auto& cv = *static_cast<ComparatorView*>(view);
	cv.BoundsEditMode = !cv.BoundsEditMode;
}


void ComparatorView::OnPot1TurnHandler(ViewBase* view, float val) {
	auto& cv = *static_cast<ComparatorView*>(view);
	if (!cv.Editable)
		return;

	auto idx = cv.BoundsEditMode ? kParamWindowLeft : kParamWindowCenter;
	float rmin = cv.Algorithm->RangeMin;
	float rmax = cv.Algorithm->RangeMax;

	auto algIndex = NT_algorithmIndex(cv.Algorithm);
	auto paramIdx = cv.Algorithm->ChannelOffsets[cv.SelectedComparatorIndex] + idx;
	auto paramVal = GetScaledParameterValue(*cv.Algorithm, paramIdx);
	cv.Algorithm->PotMgr.UpdateValueWithPot(0, val, paramVal, rmin, rmax);
	auto unscaled = UnscaleValueForParameter(*cv.Algorithm, paramIdx, paramVal);
	SetParameterValue(algIndex, paramIdx + NT_parameterOffset(), unscaled, CallingContext::UiThread);

	cv.DisplayBarStatsHelpText();
}


void ComparatorView::OnPot3TurnHandler(ViewBase* view, float val) {
	auto& cv = *static_cast<ComparatorView*>(view);
	if (!cv.Editable)
		return;

	auto idx = cv.BoundsEditMode ? kParamWindowRight : kParamWindowWidth;
	float rmin = cv.BoundsEditMode ? cv.Algorithm->RangeMin : 0;
	float rmax = cv.BoundsEditMode ? cv.Algorithm->RangeMax : cv.Algorithm->Range;

	auto algIndex = NT_algorithmIndex(cv.Algorithm);
	auto paramIdx = cv.Algorithm->ChannelOffsets[cv.SelectedComparatorIndex] + idx;
	auto paramVal = GetScaledParameterValue(*cv.Algorithm, paramIdx);
	cv.Algorithm->PotMgr.UpdateValueWithPot(2, val, paramVal, rmin, rmax);
	auto unscaled = UnscaleValueForParameter(*cv.Algorithm, paramIdx, paramVal);
	SetParameterValue(algIndex, paramIdx + NT_parameterOffset(), unscaled, CallingContext::UiThread);

	cv.DisplayBarStatsHelpText();
}


void ComparatorView::OnFixupPotValuesHandler(ViewBase* view, _NT_float3& pots) {
	// since we are using our own soft takeover logic, always return a midpoint here
	// our logic will take over from there
	pots[0] = 0.5;
	pots[1] = 0.5;
	pots[2] = 0.5;
}


ComparatorView::ComparatorView() {
	OnDraw = OnDrawHandler;
	OnEncoder1Turn = OnEncoder1TurnHandler;
	OnEncoder2ShortPress = OnEncoder2ShortPressHandler;
	OnEncoder2LongPress = OnEncoder2LongPressHandler;
	OnPot1Turn = OnPot1TurnHandler;
	OnPot3Turn = OnPot3TurnHandler;
	OnFixupPotValues = OnFixupPotValuesHandler;
}


void ComparatorView::InjectDependencies(const Dependencies& dependencies) {
	Algorithm = dependencies.Algorithm;
	ViewBase::InjectDependencies(dependencies);

  // calculate this once here rather than every draw cycle
	FirstLineY = 0;
	if (Algorithm->NumChannels < MaxDisplayedLines) {
		FirstLineY = LinePadding + (MaxDisplayedLines - Algorithm->NumChannels) * (BarHeight + LinePadding) / 2;
	}
}


void ComparatorView::DrawBullet(int x, int y, int color) const {
	NT_drawShapeI(kNT_rectangle, x, y, x + 2, y + 2, color * 0.4);
	NT_drawShapeI(kNT_line, x + 1, y, x + 1, y + 2, color);
	NT_drawShapeI(kNT_line, x, y + 1, x + 2, y + 1, color);
}


void ComparatorView::DrawComparator(uint8_t ch, uint8_t topIndex) const {
	bool selected = (ch == SelectedComparatorIndex);
	uint8_t barColor = selected ? SelectedBarColor : UnselectedBarColor;
	uint8_t color = selected ? SelectedColor : UnselectedColor;

  auto scale = static_cast<float>(BarWidth) / static_cast<float>(Algorithm->Range);

	// dim everything if we are not editable
	barColor = Editable ? barColor : barColor / 2;
	color = Editable ? color : color / 2;

	// calculate y position for this line
	uint8_t y = FirstLineY;
	y += (ch - topIndex) * (BarHeight + LinePadding);

	// draw the selection bullet
	if (selected) {
		DrawBullet(0, y + 4, color);
	}

	// draw the channel label
	NT_drawText(6, y + BarHeight - 1, ChannelLabels[ch], color);

	// draw the bar boundary
	NT_drawShapeI(kNT_box, BarLeft, y, BarLeft + BarWidth, y + BarHeight, barColor);

	// draw some ruler ticks
	for (int i = 1; i < Algorithm->Range; i++) {
		int x = i * scale;
		uint8_t tickLen = (Algorithm->RangeMin + i == 0) ? 2 : 1;
		NT_drawShapeI(kNT_line, BarLeft + x, y + 1, BarLeft + x, y + tickLen, 1);
		NT_drawShapeI(kNT_line, BarLeft + x, y + BarHeight - tickLen, BarLeft + x, y + BarHeight - 1, 1);
	}

	// get the bounds and value
	uint8_t offset = Algorithm->ChannelOffsets[ch];
	auto winLeft = GetScaledParameterValue(*Algorithm, offset + kParamWindowLeft);
	auto winRight = GetScaledParameterValue(*Algorithm, offset + kParamWindowRight);
	auto val = Algorithm->CurrentValues[ch];
	auto inWindow = (val >= winLeft && val <= winRight);

	// draw the window
	auto leftPos = static_cast<int>((winLeft - Algorithm->RangeMin) * scale);
	auto rightPos = static_cast<int>((winRight - Algorithm->RangeMin) * scale);
	NT_drawShapeI(kNT_box, BarLeft + leftPos, y, BarLeft + rightPos, y + BarHeight, color);
	NT_drawShapeI(kNT_rectangle, BarLeft + leftPos + 1, y + 1, BarLeft + rightPos - 1, y + BarHeight - 1, inWindow ? color / 2 : 0);
	if (selected) {
		auto left  = BarLeft + leftPos + 1;
		auto right = BarLeft + rightPos - 1;
		if (right >= left)
			NT_drawShapeI(inWindow ? kNT_rectangle : kNT_box, left, y + 1, right, y + BarHeight - 1, color / 2);
	}
	if (!BoundsEditMode) {
		auto left  = BarLeft + leftPos + 4;
		auto right = BarLeft + rightPos - 4;
		if (right >= left)
			NT_drawShapeI(kNT_rectangle, left, y + 4, right, y + 6, color);
	}

  // draw the value marker
	auto valuePos = BarLeft + static_cast<int>((val - Algorithm->RangeMin) * scale);
	const int* markerPalette = inWindow ? (const int[]){6, 5, 4, 3, 2} : (const int[]){2, 4, 7, 10, 15};
	for (int i = 0; i < 5; i++) {
		NT_drawShapeI(kNT_line, valuePos, y + i + 1, valuePos, y + BarHeight - i - 1, markerPalette[i]);
	}
}


void ComparatorView::DrawComparators() const {
	static constexpr uint8_t maxVisibleComparators = 4;
	auto topIndex = max(static_cast<int>(SelectedComparatorIndex) - 2, 0);
	auto visibleComparators = min(Algorithm->NumChannels, maxVisibleComparators);
	topIndex = min(topIndex, Algorithm->NumChannels - visibleComparators);
	for (int i = topIndex; i < topIndex + visibleComparators; i++) {
		DrawComparator(i, topIndex);
	}
}


void ComparatorView::DrawHelpSection() const {
	NT_drawShapeI(kNT_rectangle, 0, 50, 255, 63, 0);

	// draw the transient help text if present
	if (!Algorithm->HelpText.Draw()) {
		NT_drawText(2, 58, BoundsEditMode ? "Left" : "Center", 15, kNT_textLeft, kNT_textTiny);
		NT_drawText(33, 64, "Select Comparator", 15, kNT_textLeft, kNT_textTiny);

		if (Editable) {
			NT_drawText(170, 58, "Q: Lock", 15, kNT_textLeft, kNT_textTiny);
		} else {
			NT_drawText(166, 58, "Q: Unlock", 15, kNT_textLeft, kNT_textTiny);
		}

		if (BoundsEditMode) {
			NT_drawText(140, 64, "L: Center/Width Mode", 15, kNT_textLeft, kNT_textTiny);
		} else {
			NT_drawText(152, 64, "L: Bounds Mode", 15, kNT_textLeft, kNT_textTiny);
		}

		NT_drawText(233, 58, BoundsEditMode ? "Right" : "Width", 15, kNT_textLeft, kNT_textTiny);
	}
	NT_drawShapeI(kNT_line, 0, 50, 255, 50, 15);
}


void ComparatorView::DisplayBarStatsHelpText() {
	auto render = [](void* context) {
		auto& cv = *static_cast<ComparatorView*>(context);

		auto offset = cv.Algorithm->ChannelOffsets[cv.SelectedComparatorIndex];
		auto left = GetScaledParameterValue(*cv.Algorithm, offset + kParamWindowLeft);
		auto right = GetScaledParameterValue(*cv.Algorithm, offset + kParamWindowRight);
		auto center = GetScaledParameterValue(*cv.Algorithm, offset + kParamWindowCenter);
		auto width = GetScaledParameterValue(*cv.Algorithm, offset + kParamWindowWidth);

		char buffer[11];
		char* buf = buffer;
		strncpy(buf, "L: ", 3);
		buf += 3;
		buf += NT_floatToString(buf, left, 3);
		*buf = '\0';
		NT_drawText(10, 62, buffer);

		buf = buffer;
		strncpy(buf, "R: ", 3);
		buf += 3;
		buf += NT_floatToString(buf, right, 3);
		*buf = '\0';
		NT_drawText(70, 62, buffer);

		buf = buffer;
		strncpy(buf, "C: ", 3);
		buf += 3;
		buf += NT_floatToString(buf, center, 3);
		*buf = '\0';
		NT_drawText(140, 62, buffer);

		buf = buffer;
		strncpy(buf, "W: ", 3);
		buf += 3;
		buf += NT_floatToString(buf, width, 3);
		*buf = '\0';
		NT_drawText(200, 62, buffer);
	};

	Algorithm->HelpText.DisplayHelpCallback(render, this);
}