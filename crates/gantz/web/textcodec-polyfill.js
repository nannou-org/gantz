// GANTZ_TEXTCODEC_POLYFILL
// cpal's AudioWorklet backend loads the wasm-bindgen glue on the audio worklet thread. The glue
// constructs `TextEncoder` and `TextDecoder` at module load, but `AudioWorkletGlobalScope` lacks
// them in Firefox. This defines minimal UTF-8 implementations when they are missing. On the main
// thread the native constructors exist, so this is a no-op there.
(function (g) {
  if (typeof g.TextEncoder === "undefined") {
    g.TextEncoder = class TextEncoder {
      get encoding() {
        return "utf-8";
      }
      encode(str) {
        str = String(str);
        const out = [];
        for (let i = 0; i < str.length; i++) {
          let c = str.codePointAt(i);
          if (c > 0xffff) i++; // surrogate pair consumed
          if (c < 0x80) out.push(c);
          else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
          else if (c < 0x10000)
            out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
          else
            out.push(
              0xf0 | (c >> 18),
              0x80 | ((c >> 12) & 0x3f),
              0x80 | ((c >> 6) & 0x3f),
              0x80 | (c & 0x3f),
            );
        }
        return new Uint8Array(out);
      }
      encodeInto(str, dst) {
        const enc = this.encode(str);
        const n = Math.min(enc.length, dst.length);
        dst.set(enc.subarray(0, n));
        return { read: str.length, written: n };
      }
    };
  }
  if (typeof g.TextDecoder === "undefined") {
    g.TextDecoder = class TextDecoder {
      constructor(label, options) {
        this.encoding = "utf-8";
        this.fatal = !!(options && options.fatal);
        this.ignoreBOM = !!(options && options.ignoreBOM);
      }
      decode(input) {
        if (!input) return "";
        const b =
          input instanceof Uint8Array ? input : new Uint8Array(input.buffer || input);
        let out = "";
        for (let i = 0; i < b.length; ) {
          let c = b[i++];
          if (c >= 0xf0)
            c =
              ((c & 0x07) << 18) |
              ((b[i++] & 0x3f) << 12) |
              ((b[i++] & 0x3f) << 6) |
              (b[i++] & 0x3f);
          else if (c >= 0xe0)
            c = ((c & 0x0f) << 12) | ((b[i++] & 0x3f) << 6) | (b[i++] & 0x3f);
          else if (c >= 0xc0) c = ((c & 0x1f) << 6) | (b[i++] & 0x3f);
          out += String.fromCodePoint(c);
        }
        return out;
      }
    };
  }
})(globalThis);
