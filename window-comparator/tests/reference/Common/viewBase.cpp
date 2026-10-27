#include <distingnt/api.h>
#include "viewBase.h"


void ViewBase::InjectDependencies(const Dependencies& dependencies) {
	Timer = dependencies.Timer;
}


void ViewBase::DrawEditBox(uint8_t x, uint8_t y, uint8_t width, const char* text, bool selected, bool editable) const {
	auto allowEdit = selected && editable;
	auto backgroundColor = allowEdit ? EditBoxSelectedBackgroundColor : EditBoxUnselectedBackgroundColor;
	NT_drawShapeI(kNT_rectangle, x - 1, y, x + width + 1, y + 8, backgroundColor);
	auto textColor = allowEdit ? EditBoxSelectedTextColor : EditBoxUnselectedTextColor;
	NT_drawText(x, y + 8, text, textColor);
	if (allowEdit) {
		NT_drawShapeI(kNT_box, x - 2, y - 1, x + width + 2, y + 9, EditBoxSelectedBorderColor);
	}
}


void ViewBase::ProcessControlInput(const _NT_uiData& data) {

	// lambda for encoder turn functionality
	auto handleEncoderTurn = [&](int index, auto turnCallback) {
    if (data.encoders[index] && turnCallback) {
			turnCallback(this, data.encoders[index]);
		}
	};
	handleEncoderTurn(0, OnEncoder1Turn);
	handleEncoderTurn(1, OnEncoder2Turn);

	// lambda for pot turn functionality
	auto handlePotTurn = [&](uint32_t mask, int index, auto turnCallback) {
    if ((data.controls & mask) && turnCallback) {
			// don't register turns during the brief period where we are lifting our finger after a press
			if (PotDownTime[index] == 0 && BlockPotChangesUntil[index] <= Timer->TotalMs) {
					turnCallback(this, data.pots[index]);
			}
    }
	};
	handlePotTurn(kNT_potL, 0, OnPot1Turn);
	handlePotTurn(kNT_potC, 1, OnPot2Turn);
	handlePotTurn(kNT_potR, 2, OnPot3Turn);

	// lambda for encoder push functionality
	auto handleEncoderPush = [&](uint32_t mask, int index, auto pushCallback) {
    if ((data.controls & mask) && !(data.lastButtons & mask)) {
			EncoderDownTime[index] = Timer->TotalMs;
			if (pushCallback)
				pushCallback(this);
    }
	};
	handleEncoderPush(kNT_encoderButtonL, 0, OnEncoder1Push);
	handleEncoderPush(kNT_encoderButtonR, 1, OnEncoder2Push);

	// lambda for encoder release functionality
	auto handleEncoderRelease = [&](uint32_t mask, int index, auto shortPressCallback, auto releaseCallback) {
		if (!(data.controls & mask) && (data.lastButtons & mask)) {
			if (EncoderDownTime[index] > 0) {
				// Calculate how long we held the encoder down (in ms)
				auto totalDownTime = Timer->TotalMs - EncoderDownTime[index];

				if (totalDownTime < ShortPressThreshold) {
					if (shortPressCallback)
						shortPressCallback(this);
				} else {
					// Reset to prepare for another long press
					EncoderLongPressFired[index] = false;
				}
				EncoderDownTime[index] = 0;
			}

			if (releaseCallback)
				releaseCallback(this);
		}
	};
	handleEncoderRelease(kNT_encoderButtonL, 0, OnEncoder1ShortPress, OnEncoder1Release);
	handleEncoderRelease(kNT_encoderButtonR, 1, OnEncoder2ShortPress, OnEncoder2Release);

	// lambda for pot push functionality
	auto handlePotPush = [&](uint32_t mask, int index, auto pushCallback) {
		if ((data.controls & mask) && !(data.lastButtons & mask)) {
			PotDownTime[index] = Timer->TotalMs;
			if (pushCallback)
				pushCallback(this);
		}
	};
	handlePotPush(kNT_potButtonL, 0, OnPot1Push);
	handlePotPush(kNT_potButtonC, 1, OnPot2Push);
	handlePotPush(kNT_potButtonR, 2, OnPot3Push);

	// lmabda for pot release functionality
	auto handlePotRelease = [&](uint32_t mask, int index, auto shortPressCallback, auto releaseCallback) {
		if (!(data.controls & mask) && (data.lastButtons & mask)) {
			if (PotDownTime[index] > 0) {
				auto totalDownTime = Timer->TotalMs - PotDownTime[index];
				
				if (totalDownTime < ShortPressThreshold) {
					if (shortPressCallback)
						shortPressCallback(this);
				} else {
					PotLongPressFired[index] = false;
				}

				PotDownTime[index] = 0;
				BlockPotChangesUntil[index] = Timer->TotalMs + 100;
			}

			if (releaseCallback)
				releaseCallback(this);
		}
	};
	handlePotRelease(kNT_potButtonL, 0, OnPot1ShortPress, OnPot1Release);
	handlePotRelease(kNT_potButtonC, 1, OnPot2ShortPress, OnPot2Release);
	handlePotRelease(kNT_potButtonR, 2, OnPot3ShortPress, OnPot3Release);

	// lambda for button push functionality
	auto handleButtonPush = [&](uint32_t mask, auto pushCallback) {
		if ((data.controls & mask) && !(data.lastButtons & mask) && pushCallback) {
			pushCallback(this);
		}
	};
	handleButtonPush(kNT_button1, OnButton1Push);
	handleButtonPush(kNT_button2, OnButton2Push);
	handleButtonPush(kNT_button3, OnButton3Push);
	handleButtonPush(kNT_button4, OnButton4Push);

	// lambda for button release functionality
	auto handleButtonRelease = [&](uint32_t mask, auto releaseCallback) {
		if (!(data.controls & mask) && (data.lastButtons & mask) && releaseCallback) {
			releaseCallback(this);
		}
	};
	handleButtonRelease(kNT_button1, OnButton1Release);
	handleButtonRelease(kNT_button2, OnButton2Release);
	handleButtonRelease(kNT_button3, OnButton3Release);
	handleButtonRelease(kNT_button4, OnButton4Release);
}


// this method needs to be called regularly in order to measure time and fire the long press
// generally calling it from draw() is a good idea, because you don't need as frequently as step()
void ViewBase::ProcessLongPresses() {

	// lambda for handling pots
	auto handlePotLongPress = [&](int index, auto longPressCallback) {
		if (PotDownTime[index] > 0 && !PotLongPressFired[index]) {
			// Calculate how long we held the pot down (in ms)
			auto totalDownTime = Timer->TotalMs - PotDownTime[index];
			
			if (totalDownTime >= ShortPressThreshold) {
				if (longPressCallback)
					longPressCallback(this);
						
				PotLongPressFired[index] = true;
			}
		}
	};

	handlePotLongPress(0, OnPot1LongPress);
	handlePotLongPress(1, OnPot2LongPress);
	handlePotLongPress(2, OnPot3LongPress);

	// lambda for handling encoders
	auto handleEncoderLongPress = [&](int index, auto longPressCallback) {
    if (EncoderDownTime[index] > 0 && !EncoderLongPressFired[index]) {
			// Calculate how long we held the encoder down (in ms)
			auto totalDownTime = Timer->TotalMs - EncoderDownTime[index];
			
			if (totalDownTime >= ShortPressThreshold) {
				if (longPressCallback)
					longPressCallback(this);
						
				EncoderLongPressFired[index] = true;
			}
    }
	};

	handleEncoderLongPress(0, OnEncoder1LongPress);
	handleEncoderLongPress(1, OnEncoder2LongPress);
}


void ViewBase::Activate() {
	if (OnActivate)
		OnActivate(this);
}


void ViewBase::Draw() {
	ProcessLongPresses();
	if (OnDraw)
		OnDraw(this);
}


void ViewBase::FixupPotValues(_NT_float3& pots) {
	if (OnFixupPotValues)
		OnFixupPotValues(this, pots);
}


void ViewBase::ParameterChanged(int paramIndex) {
	if (OnParameterChanged)
		OnParameterChanged(this, paramIndex);
}
