(() => {
    "use strict";

    const storageKey = "urlharvest-theme";
    const savedTheme = (() => {
        try {
            const theme = localStorage.getItem(storageKey);
            return theme === "light" || theme === "dark" ? theme : "system";
        } catch (_) {
            return "system";
        }
    })();

    if (savedTheme !== "system") {
        document.documentElement.dataset.theme = savedTheme;
    }

    const initializeSelector = () => {
        const selector = document.getElementById("theme-select");
        if (!selector) {
            return;
        }

        selector.value = savedTheme;
        selector.addEventListener("change", () => {
            const theme = selector.value;
            if (theme === "system") {
                document.documentElement.removeAttribute("data-theme");
            } else {
                document.documentElement.dataset.theme = theme;
            }

            try {
                if (theme === "system") {
                    localStorage.removeItem(storageKey);
                } else {
                    localStorage.setItem(storageKey, theme);
                }
            } catch (_) { }
        });
    };

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", initializeSelector, { once: true });
    } else {
        initializeSelector();
    }
})();
