#include <distingnt/api.h>
#include "helpTextHelper.h"


void HelpTextHelper::DisplayHelpText(int xPos, const char* text) {
	HelpText = text;
	RemainingDuration = DurationFrames;
	xPosition = xPos;
}


void HelpTextHelper::DisplayHelpCallback(void (*drawCallback)(void*), void* context) {
	DrawCallback = drawCallback;
	Context = context;
	RemainingDuration = DurationFrames;
}


bool HelpTextHelper::Draw() {
	if (HelpText) {
		NT_drawText(xPosition, 62, HelpText);
		RemainingDuration--;
		if (RemainingDuration <= 0) {
			HelpText = NULL;
		}
		return true;
	}
	
	if (DrawCallback) {
		DrawCallback(Context);
		RemainingDuration--;
		if (RemainingDuration <= 0) {
			DrawCallback = NULL;
		}
		return true;
	}

	return false;
}