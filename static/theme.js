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
        const controls = document.querySelectorAll('input[name="theme"]');
        if (controls.length === 0) {
            return;
        }

        for (const control of controls) {
            control.checked = control.value === savedTheme;
            control.addEventListener("change", () => {
                if (!control.checked) {
                    return;
                }

                const theme = control.value;
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
        }
    };

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", initializeSelector, { once: true });
    } else {
        initializeSelector();
    }
})();
