// Custom trunk initializer that drives the loading progress bar and filters
// Bevy's WASM exception-based control flow errors.
export default function initializer() {
    const loading = document.getElementById("loading");
    const bar = document.getElementById("loading-bar");
    return {
        onStart: () => {},
        onProgress: ({current, total}) => {
            if (bar && total > 0) {
                bar.style.width = `${Math.round((current / total) * 100)}%`;
            }
        },
        onComplete: () => {
            loading?.remove();
        },
        onSuccess: (_wasm) => {},
        onFailure: (error) => {
            if (!error.message?.startsWith("Using exceptions for control flow (not an error)")) {
                if (loading) {
                    loading.innerHTML = "<p>Failed to load app. See console for details.</p>";
                }
                throw error;
            }
        },
    };
}
