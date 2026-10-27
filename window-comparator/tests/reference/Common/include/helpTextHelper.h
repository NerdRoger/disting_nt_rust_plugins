#pragma once
#include <stddef.h>

struct HelpTextHelper {
	static constexpr int DurationFrames = 90;

	const char* HelpText = NULL;
	int RemainingDuration = 0;
	int xPosition = 0;
	void (*DrawCallback)(void* context) = NULL;
	void* Context;

	void DisplayHelpText(int xPos, const char* text);
	void DisplayHelpCallback(void (*drawCallback)(void*), void* context);
	bool Draw();
};
