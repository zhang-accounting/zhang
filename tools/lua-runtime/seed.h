// The string-hash seed of every Lua state, included ahead of each C++ translation unit of the module (build.py
// passes `-include seed.h`), so lstate.c finds `luai_makeseed` defined and skips its own. Lua's default mixes the
// clock's seconds with three addresses, and in wasm the addresses are the same on every run: a script, or the JSON
// a host answer carries, could be crafted with keys that all collide in one hash chain (hash flooding). Each
// lua_newstate takes 32 random bits from WASI instead; the clock's nanoseconds stand in should the host refuse
// randomness. Nothing depends on the order `pairs` visits keys in: dkjson writes an object's keys sorted
// (vendor/README.md).
#ifndef ZHANG_LUA_SEED_H
#define ZHANG_LUA_SEED_H

#include <time.h>
#include <unistd.h>

static inline unsigned int zhang_random_seed(void) {
  unsigned int seed;
  if (getentropy(&seed, sizeof seed) == 0) return seed;
  struct timespec now;
  clock_gettime(CLOCK_REALTIME, &now);
  return (unsigned int)now.tv_nsec ^ (unsigned int)now.tv_sec;
}

#define luai_makeseed(L) zhang_random_seed()

#endif
