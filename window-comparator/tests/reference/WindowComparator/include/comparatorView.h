#pragma once
#include <distingnt/api.h>
#include "common.h"
#include "viewBase.h"


struct WindowComparatorAlg;


struct ComparatorView : ViewBase {
public:
	struct Dependencies : ViewBase::Dependencies {
		WindowComparatorAlg* Algorithm = nullptr;
	};

private:

	friend struct WindowComparatorAlg;

	WindowComparatorAlg* Algorithm = nullptr;

	uint8_t SelectedComparatorIndex = 0;
	uint8_t FirstLineY = 0;
	bool BoundsEditMode = true;
	bool Editable = true;

	void DrawBullet(int x, int y, int color) const;
	void DrawComparator(uint8_t ch, uint8_t topIndex) const;
	void DrawComparators() const;
	void DrawHelpSection() const;
	void DisplayBarStatsHelpText();

	HIDDEN static void OnDrawHandler(ViewBase* view);
	HIDDEN static void OnEncoder1TurnHandler(ViewBase* view, int8_t x);
	HIDDEN static void OnEncoder2ShortPressHandler(ViewBase* view);
	HIDDEN static void OnEncoder2LongPressHandler(ViewBase* view);
	HIDDEN static void OnPot1TurnHandler(ViewBase* view, float val);
	HIDDEN static void OnPot3TurnHandler(ViewBase* view, float val);
	HIDDEN static void OnFixupPotValuesHandler(ViewBase* view, _NT_float3& pots);

public:
	ComparatorView();
	void InjectDependencies(const Dependencies& dependencies);
};