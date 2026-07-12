#pragma once
// meshoptimizer only uses printf for debug output — no-op on WASM.
#define printf(...) ((void)0)
#define fprintf(...) ((void)0)
