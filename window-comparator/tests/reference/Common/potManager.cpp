#include <stdlib.h>
#include "potManager.h"


void PotManager::RecordPreviousPotValues(const _NT_uiData& data) {
	for (size_t i = 0; i < ARRAY_SIZE(data.pots); i++) {
		PreviousPotValues[i] = data.pots[i];
	}
	HasPreviousPotValues = true;
}


void PotManager::UpdateValueWithPot(int potIndex, float currentPotVal, float& value, float min, float max) {
	// if we have not yet recorded previous pot values, ignore this very first pot move
	// this will "prime the pump" and stop large jumps in values on the very first move
	if (!HasPreviousPotValues) {
		return;
	}

	auto prevPotVal = PreviousPotValues[potIndex];

	// get the change in pot position since our last known value
	auto dx = currentPotVal - prevPotVal;

	// if it has changed too much (more than 1%), it probably happened when we were unaware of it, in another Disting screen.
	// so just return the known value, and the next call should get us back on track, once our tracking is back in sync
	if (abs(dx) > 0.01) {
		return;
	}

	// are we increasing or decreasing the pot?
	bool increasing = (currentPotVal > prevPotVal);
	
	// if we're very close to an extreme, go ahead and set the value to the relevant min or max
	// this means we will end at 10 instead of 9.99973 for example, or 0 instead of 0.00042
	// only do this when moving toward the extreme, as doing it when moving away just keeps attracting the value back
	if (increasing && (currentPotVal > 0.995)) {
		value = max;
		return;
	}
	if (!increasing && currentPotVal < 0.005) {
		value = min;
		return;
	}

	// otherwise, soft takeover logic
	auto valRange = increasing ? max - value : value - min;
	auto potRange = increasing ? 1 - prevPotVal : prevPotVal;
	auto factor = valRange / potRange;
	value += (factor * dx);
}
