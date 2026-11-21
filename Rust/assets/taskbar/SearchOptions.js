(() => {
    const styleId = "windhawk-windows-11-start-menu-styler-style";
    const style = document.getElementById(styleId);
    if (!style) return;
    style.dataset.smartTaskbarSearchOptions = "true";

    /* Filter the dropdown model before rendering so keyboard navigation skips the hidden entries. */
    const installed = Symbol.for("SmartTaskbar.SearchOptions");
    function install() {
        const menu = window.WSB?.MenuFactory?.prototype;
        if (typeof menu?.showDropdownMenu !== "function") return false;
        if (menu[installed]) return true;

        const showDropdownMenu = menu.showDropdownMenu;
        menu.showDropdownMenu = function (items, button, menuType, ...rest) {
            const active = document.getElementById(styleId)?.dataset.smartTaskbarSearchOptions;
            const host = window.WSB?.Host;
            if (active && button?.id === "optionsButton" && menuType === 2 &&
                Array.isArray(items) && typeof host?.getLocString === "function") {
                try {
                    const prompts = new Set([
                        "EnableMsaSearch", "EnableAadSearch", "AddMicrosoftAccount", "AddAadAccount",
                    ].map(key => host.getLocString(key)).filter(Boolean));

                    function filterPrompts(options) {
                        return options.filter(item => !prompts.has(item.text)).map(item => {
                            if (!item.subItems) return item;
                            const children = filterPrompts(item.subItems);
                            return children.every((child, index) => child === item.subItems[index]) &&
                                children.length === item.subItems.length
                                ? item : { ...item, subItems: children };
                        });
                    }

                    items = filterPrompts(items);
                } catch {
                    /* A different Windows Search layout should still open the original menu. */
                }
            }
            return showDropdownMenu.call(this, items, button, menuType, ...rest);
        };
        menu[installed] = true;
        return true;
    }

    if (!install()) window.addEventListener("load", install, { once: true });
})();
