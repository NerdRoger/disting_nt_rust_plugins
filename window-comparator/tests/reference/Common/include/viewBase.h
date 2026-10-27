#pragma once
#include <stdint.h>
#include <distingnt/api.h>
#include "timeKeeper.h"


struct ViewBase {
public:
	struct Dependencies {
		TimeKeeper* Timer = nullptr;
	};

private:
	static constexpr int EditBoxUnselectedBackgroundColor = 1;
	static constexpr int EditBoxSelectedBackgroundColor = 1;
	static constexpr int EditBoxUnselectedTextColor = 8;
	static constexpr int EditBoxSelectedTextColor = 15;
	static constexpr int EditBoxSelectedBorderColor = 15;

	static constexpr uint16_t ShortPressThreshold = 250; // How long (in ms) until a short press turns into a long press	
	
	TimeKeeper* Timer = nullptr;

	uint32_t BlockPotChangesUntil[3];
	uint32_t PotDownTime[3];
	bool PotLongPressFired[3];

	uint32_t EncoderDownTime[2];
	bool EncoderLongPressFired[2];

	void ProcessLongPresses();

protected:
	mutable char NumToStrBuf[20]; // for storing conversion results

	void DrawEditBox(uint8_t x, uint8_t y, uint8_t width, const char* text, bool selected, bool editable) const;
	void InjectDependencies(const Dependencies& dependencies);

public:

	void ProcessControlInput(const _NT_uiData& data);

	void (*OnActivate)(ViewBase* view);
	void (*OnDraw)(ViewBase* view);

	void (*OnEncoder1Turn)(ViewBase* view, int8_t x);
	void (*OnEncoder1Push)(ViewBase* view);
	void (*OnEncoder1Release)(ViewBase* view);
	void (*OnEncoder1ShortPress)(ViewBase* view);
	void (*OnEncoder1LongPress)(ViewBase* view);

	void (*OnEncoder2Turn)(ViewBase* view, int8_t x);
	void (*OnEncoder2Push)(ViewBase* view);
	void (*OnEncoder2Release)(ViewBase* view);
	void (*OnEncoder2ShortPress)(ViewBase* view);
	void (*OnEncoder2LongPress)(ViewBase* view);

	void (*OnPot1Turn)(ViewBase* view, float val);
	void (*OnPot1Push)(ViewBase* view);
	void (*OnPot1Release)(ViewBase* view);
	void (*OnPot1ShortPress)(ViewBase* view);
	void (*OnPot1LongPress)(ViewBase* view);
	
	void (*OnPot2Turn)(ViewBase* view, float val);
	void (*OnPot2Push)(ViewBase* view);
	void (*OnPot2Release)(ViewBase* view);
	void (*OnPot2ShortPress)(ViewBase* view);
	void (*OnPot2LongPress)(ViewBase* view);
	
	void (*OnPot3Turn)(ViewBase* view, float val);
	void (*OnPot3Push)(ViewBase* view);
	void (*OnPot3Release)(ViewBase* view);
	void (*OnPot3ShortPress)(ViewBase* view);
	void (*OnPot3LongPress)(ViewBase* view);
	
	void (*OnButton1Push)(ViewBase* view);
	void (*OnButton1Release)(ViewBase* view);
	
	void (*OnButton2Push)(ViewBase* view);
	void (*OnButton2Release)(ViewBase* view);
	
	void (*OnButton3Push)(ViewBase* view);
	void (*OnButton3Release)(ViewBase* view);
	
	void (*OnButton4Push)(ViewBase* view);
	void (*OnButton4Release)(ViewBase* view);
	
	void (*OnFixupPotValues)(ViewBase* view, _NT_float3& pots);
	void (*OnParameterChanged)(ViewBase* view, int paramIndex);

	void FixupPotValues(_NT_float3& pots);
	void ParameterChanged(int paramIndex);
	void Activate();
	void Draw();

};
