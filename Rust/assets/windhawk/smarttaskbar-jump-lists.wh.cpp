// ==WindhawkMod==
// @id              smarttaskbar-jump-lists
// @name            SmartTaskbar Popup Anchors
// @description     Anchor native app menus and tray overflow before drawing
// @version         1.1
// @author          SmartTaskbar contributors
// @include         explorer.exe
// @architecture    x86-64
// ==/WindhawkMod==
//
// SPDX-License-Identifier: MIT

// ==WindhawkModSettings==
/*
- snapToAppButton: true
  $name: Place Jump Lists at the clicked app button
*/
// ==/WindhawkModSettings==

#include <windows.h>
#include <windowsx.h>
#include <windhawk_utils.h>
#include <algorithm>
#include <atomic>
#include <cstdint>

namespace {

constexpr wchar_t kAnchor[] = L"SmartTaskbar.Rust.JumpList.Anchor";
constexpr wchar_t kRequestedAt[] = L"SmartTaskbar.Rust.JumpList.RequestedAt";
constexpr ULONGLONG kRequestLifetime = 1200;
std::atomic_bool enabled;
thread_local bool computingJumpList;

// Only the ABI of these two Windows.Foundation value types is needed.
struct XamlPoint { float x, y; };
struct XamlRect { float x, y, width, height; };
using ComputeJumpPosition = HRESULT(WINAPI*)(void*, void*, int, XamlPoint*, int*, int*);
using ToDesktopPixels = RECT(WINAPI*)(const XamlRect&, UINT, HWND);
ComputeJumpPosition originalCompute;
ToDesktopPixels originalToPixels;

HRESULT WINAPI ComputeHook(void* self, void* group, int reason, XamlPoint* position,
                          int* horizontalAlignment, int* verticalAlignment) {
    const bool previous = computingJumpList;
    computingJumpList = enabled.load(std::memory_order_relaxed);
    const auto result = originalCompute(self, group, reason, position,
                                        horizontalAlignment, verticalAlignment);
    computingJumpList = previous;
    return result;
}

RECT WINAPI ToPixelsHook(const XamlRect& rect, UINT dpi, HWND window) {
    auto native = originalToPixels(rect, dpi, window);
    if (!computingJumpList || native.left >= native.right || native.top >= native.bottom) {
        return native;
    }
    const HWND bar = GetAncestor(window, GA_ROOT);
    const auto requestedAt = reinterpret_cast<uintptr_t>(GetPropW(bar, kRequestedAt));
    const auto packed = reinterpret_cast<uintptr_t>(GetPropW(bar, kAnchor));
    const auto now = GetTickCount64();
    if (!packed || requestedAt > now || now - requestedAt >= kRequestLifetime) {
        return native;
    }
    const POINT button{static_cast<int32_t>(packed), static_cast<int32_t>(packed >> 32)};
    RECT bounds{};
    if (!GetWindowRect(bar, &bounds) || !PtInRect(&bounds, button)) {
        return native;
    }
    if (bounds.right - bounds.left > bounds.bottom - bounds.top) {
        OffsetRect(&native, button.x - (native.left + (native.right - native.left) / 2), 0);
    } else {
        OffsetRect(&native, 0, button.y - (native.top + (native.bottom - native.top) / 2));
    }
    return native;
}

constexpr wchar_t kIslandSpan[] = L"SmartTaskbarRust.31a9.IslandSpan";
using SetPosition = decltype(&SetWindowPos);
SetPosition originalSetPosition;

struct OverflowAnchor {
    HWND popup;
    HWND bar;
    POINT button;
};
thread_local OverflowAnchor overflowAnchor;

HWND IslandAt(POINT point) {
    const HWND bar = GetAncestor(WindowFromPoint(point), GA_ROOT);
    // The controller publishes this only when it owns a measured island.
    return bar && GetPropW(bar, kIslandSpan) ? bar : nullptr;
}

bool PlaceOverflow(const OverflowAnchor& anchor, int width, int height, int& x, int& y) {
    RECT bar{};
    MONITORINFO monitor{.cbSize = sizeof(MONITORINFO)};
    if (!anchor.bar || !GetPropW(anchor.bar, kIslandSpan) ||
        !GetWindowRect(anchor.bar, &bar) ||
        !GetMonitorInfoW(MonitorFromWindow(anchor.bar, MONITOR_DEFAULTTONULL), &monitor)) {
        return false;
    }
    const RECT display = monitor.rcMonitor;
    if (width <= 0 || height <= 0 || width > display.right - display.left ||
        height > display.bottom - display.top) {
        return false;
    }

    const int gap = MulDiv(4, GetDpiForWindow(anchor.bar), 96);
    if (bar.right - bar.left >= bar.bottom - bar.top) {
        const bool top = bar.top - display.top < display.bottom - bar.bottom;
        x = std::clamp<int>(anchor.button.x - width / 2, display.left, display.right - width);
        y = top ? bar.bottom + gap : bar.top - gap - height;
        return y >= display.top && y <= display.bottom - height;
    }
    const bool left = bar.left - display.left < display.right - bar.right;
    x = left ? bar.right + gap : bar.left - gap - width;
    y = std::clamp<int>(anchor.button.y - height / 2, display.top, display.bottom - height);
    return x >= display.left && x <= display.right - width;
}

BOOL WINAPI SetPositionHook(HWND popup, HWND after, int x, int y, int width, int height,
                            UINT flags) {
    const auto original = [&] {
        return originalSetPosition(popup, after, x, y, width, height, flags);
    };
    if (!enabled.load(std::memory_order_relaxed) ||
        ((flags & SWP_NOMOVE) && !(flags & SWP_SHOWWINDOW))) {
        return original();
    }
    wchar_t name[48];
    if (!GetClassNameW(popup, name, ARRAYSIZE(name)) ||
        (wcscmp(name, L"TopLevelWindowForOverflowXamlIsland") != 0 &&
         wcscmp(name, L"NotifyIconOverflowWindow") != 0)) {
        return original();
    }

    // Explorer opens overflow on the thread that handles the arrow click.
    // Keep that location during layout changes while the menu is open.
    if (overflowAnchor.popup != popup || !IsWindowVisible(popup)) {
        const auto message = GetMessagePos();
        POINT point{GET_X_LPARAM(message), GET_Y_LPARAM(message)};
        HWND bar = IslandAt(point);
        if (!bar && GetCursorPos(&point)) {
            bar = IslandAt(point);
        }
        overflowAnchor = {popup, bar, point};
    }
    int popupWidth = width;
    int popupHeight = height;
    if (flags & SWP_NOSIZE) {
        RECT current{};
        if (!GetWindowRect(popup, &current)) {
            return original();
        }
        popupWidth = current.right - current.left;
        popupHeight = current.bottom - current.top;
    }
    int anchoredX;
    int anchoredY;
    if (!PlaceOverflow(overflowAnchor, popupWidth, popupHeight, anchoredX, anchoredY)) {
        return original();
    }
    x = anchoredX;
    y = anchoredY;
    flags &= ~SWP_NOMOVE;
    return original();
}

} // namespace

BOOL Wh_ModInit() {
    enabled = Wh_GetIntSetting(L"snapToAppButton") != 0;
    const auto taskbar = GetModuleHandleW(L"Taskbar.dll");
    if (!taskbar) {
        return FALSE;
    }
    // Resolve the native anchor calculation and its screen-coordinate conversion.
    const WindhawkUtils::SYMBOL_HOOK hooks[] = {
        {{LR"(protected: long __cdecl CTaskListWnd::_ComputeJumpViewPosition(struct ITaskBtnGroup *,int,struct Windows::Foundation::Point &,enum Windows::UI::Xaml::HorizontalAlignment &,enum Windows::UI::Xaml::VerticalAlignment &)const )"},
         &originalCompute, ComputeHook},
        {{LR"(struct tagRECT __cdecl ToRawPixelsInDesktopCoordinates(struct winrt::Windows::Foundation::Rect const &,unsigned int,struct HWND__ *))"},
         &originalToPixels, ToPixelsHook},
    };
    return WindhawkUtils::HookSymbols(taskbar, hooks, ARRAYSIZE(hooks)) &&
           WindhawkUtils::SetFunctionHook(SetWindowPos, SetPositionHook, &originalSetPosition);
}

void Wh_ModSettingsChanged() {
    enabled = Wh_GetIntSetting(L"snapToAppButton") != 0;
}
