// Custom trunk initializer that filters Bevy's WASM exception-based control flow errors.
export default function initializer() {
    return {
        onStart: () => {},
        onProgress: ({current, total}) => {},
        onComplete: () => {},
        onSuccess: (_wasm) => {},
        onFailure: (error) => {
            if (!error.message?.startsWith("Using exceptions for control flow (not an error)")) {
                throw error;
            }
        },
    };
}
