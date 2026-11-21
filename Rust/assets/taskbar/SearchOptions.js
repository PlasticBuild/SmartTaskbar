(function () {
    /* Native offline Search can still run on EdgeHTML. Keep the script compatible with ES5. */
    var styleId = "windhawk-windows-11-start-menu-styler-style";
    var marker = "data-smart-taskbar-search-options";
    var style = document.getElementById(styleId);
    if (!style) return;
    style.setAttribute(marker, "true");

    var installed = "SmartTaskbar.SearchOptions";
    function install() {
        var factory = window.WSB && window.WSB.MenuFactory;
        if (!factory || typeof factory.prototype.showDropdownMenu !== "function") return false;
        var menu = factory.prototype;
        if (menu[installed]) return true;

        /* Filter before rendering so keyboard navigation never selects a hidden entry. */
        var showDropdownMenu = menu.showDropdownMenu;
        menu.showDropdownMenu = function (items, button, menuType) {
            var activeStyle = document.getElementById(styleId);
            if (!activeStyle || !activeStyle.hasAttribute(marker) || !button ||
                button.id !== "optionsButton" || menuType !== 2 || !Array.isArray(items)) {
                return showDropdownMenu.apply(this, arguments);
            }

            var labels = [
                "Turn on cloud search for your Microsoft account",
                "Turn on cloud search for your work or school account"
            ];
            var host = window.WSB && window.WSB.Host;
            if (host && typeof host.getLocString === "function") {
                try {
                    ["EnableMsaSearch", "EnableAadSearch", "AddMicrosoftAccount", "AddAadAccount"]
                        .forEach(function (key) {
                            var text = host.getLocString(key);
                            if (text) labels.push(text);
                        });
                } catch (error) {
                    /* Keep the English fallback if the localization interface is unavailable. */
                }
            }
            var args = Array.prototype.slice.call(arguments);
            args[0] = filterPrompts(items, labels);
            return showDropdownMenu.apply(this, args);
        };
        menu[installed] = true;
        return true;
    }

    function filterPrompts(options, labels) {
        return options.filter(function (item) {
            return labels.indexOf(item.text) === -1;
        }).map(function (item) {
            if (!Array.isArray(item.subItems)) return item;
            var children = filterPrompts(item.subItems, labels);
            if (children.length === item.subItems.length && children.every(function (child, index) {
                return child === item.subItems[index];
            })) return item;
            var copy = {};
            Object.keys(item).forEach(function (key) { copy[key] = item[key]; });
            copy.subItems = children;
            return copy;
        });
    }

    if (!install()) {
        document.addEventListener("DOMContentLoaded", install);
        window.addEventListener("load", install);
    }
})();
