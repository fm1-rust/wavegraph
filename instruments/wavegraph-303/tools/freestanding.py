"""Adapt the pinned Cpp2Rust unsafe-model output to core and libm. Read stdin, write stdout."""

import re
import sys


def freestanding(source):
    # The emitted globals all correspond to C++ constexpr data. Some initializers
    # reference another constant, so replace reads before replacing definitions.
    source = source[source.index("pub static mut kPi_"):]
    source = source[:source.index("pub unsafe fn __cpp2rust_init_globals()")]
    source = re.sub(
        r"\(\*std::cell::LazyCell::force_mut\(\s*&mut \*&raw mut (\w+),?\s*\)\)",
        r"\1",
        source,
    )
    source = re.sub(
        r"(pub )?static mut (\w+): std::cell::LazyCell<([^>]+)> =\s*"
        r"std::cell::LazyCell::new\(\|\| unsafe \{\s*(.*?)\s*\}\);",
        r"\1const \2: \3 = \4;",
        source,
        flags=re.DOTALL,
    )

    # No code in these headers calls through C varargs or function-pointer
    # arguments. Preserve the Rust derives and drop the two runtime derives.
    source = re.sub(r", (?:VaArg|FnPtrArg)\b", "", source)
    source = source.replace("std::", "core::")
    source = source.replace("'loop_: while", "while").replace("'switch: {", "{")

    # The old value is consumed in the three counter tests. The other
    # postfix operations here discard it. Match libcc2rs's wrapping arithmetic.
    source = re.sub(
        r"if \(\((self\.\w+)\.postfix_dec\(\)\) <= \(0\)\) \{",
        r"let previous = \1;\n\1 = previous.wrapping_sub(1);\nif previous <= 0 {",
        source,
    )
    source = re.sub(r"([\w.]+)\.postfix_inc\(\)", r"\1 = \1.wrapping_add(1)", source)
    source = re.sub(r"([\w.]+)\.postfix_dec\(\);", r"\1 = \1.wrapping_sub(1);", source)
    source = source.replace(
        "if ((*self.pos.prefix_inc()) >= (self.size)) {",
        "self.pos = self.pos.wrapping_add(1);\nif self.pos >= self.size {",
    )

    # Reverb::Init uses one local lambda. Capture its two input fields by
    # value, and the running offset by mutable borrow, for the six call sites.
    start = source.index("let mut take: FnPtr<")
    body = source.index("                (*l).size =", start)
    end = source.index("\n            }\n        );", body)
    closure = source[body:end].replace("(*this_).k_", "scale").replace("(*this_).mem_", "memory")
    closure = closure.replace("(*at)", "at")
    source = (source[:start] + "let scale = self.k_;\nlet memory = self.mem_;\n"
              + "let mut take = |l: *mut x0x_Reverb_Line, base: f32| {\n" + closure
              + "\n};" + source[end + len("\n            }\n        );"):])
    source = source.replace("take.call(", "take(")

    # Resolve the C math declarations that Cpp2Rust leaves as external names.
    for function in ("expf", "fabsf", "floorf", "fmaxf", "fminf", "ldexpf", "log2f", "powf", "tanf"):
        source = re.sub(rf"\b{function}_\d+\(", f"libm::{function}(", source)
    return "// Generated with Cpp2Rust. See README.md for the source and reproduction steps.\n" + source


if __name__ == "__main__":
    sys.stdout.write(freestanding(sys.stdin.read()))
