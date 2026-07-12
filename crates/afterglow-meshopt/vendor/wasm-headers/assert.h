// Minimal freestanding headers for compiling meshoptimizer to wasm32-unknown-unknown.
// These replace glibc headers that NixOS's wrapped clang pulls in.
#pragma once
#define assert(x) ((void)0)
