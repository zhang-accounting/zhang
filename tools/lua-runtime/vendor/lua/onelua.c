/*
** onelua.c -- Lua core, libraries, and interpreter as a single
** translation unit. This is the monolith: the whole program compiles
** as one file, `static` is the encapsulation boundary, and the
** artifact's export list is the API. Follows the one.c/onelua.c
** pattern that upstream Lua itself ships.
**
** Build modes (default is the full `lua` interpreter):
**   -DMAKE_LIB   just the core + libraries (for embedding)
**   -DMAKE_LUA   the standalone interpreter (default)
*/

#ifndef MAKE_LIB
#ifndef MAKE_LUA
#define MAKE_LUA
#endif
#endif

/* setup for luaconf.h */
#define LUA_CORE
#define LUA_LIB
#define ltable_c
#define lvm_c
#include "luaconf.h"

/* Do not export internal symbols -- unless this build links the ltests
** debug library (-DLUA_LTESTS). ltests.c pokes VM internals by design,
** so that build leaves LUAI_FUNC at luaconf.h's plain extern. Hidden
** visibility still keeps everything out of the artifact's export list. */
#if !defined(LUA_LTESTS)
#undef LUAI_FUNC
#undef LUAI_DDEC
#undef LUAI_DDEF
#if defined(__cplusplus)
/* C++ reads an uninitialized static-const declaration as a definition,
   so const data can't be forward-declared static; extern + hidden
   visibility keeps it internal instead */
#define LUAI_FUNC	static
#define LUAI_DDEC(dec)	extern dec
#define LUAI_DDEF	/* empty */
#else
#define LUAI_FUNC	static
#define LUAI_DDEC(dec)	static dec
#define LUAI_DDEF	static
#endif
#endif /* !LUA_LTESTS */

/*
** ── WASI support ────────────────────────────────────────────────────
** wasm32-wasi lacks two things Lua needs from a C runtime:
**
** 1. Non-local jumps (Lua's error handling). Raw wasm has neither
**    setjmp/longjmp nor unwinding; both lower onto the wasm
**    exception-handling proposal. Two routes, one per language mode:
**
**    C++ (the route that works on today's toolchains): Lua natively
**    switches LUAI_THROW/LUAI_TRY to throw/catch when compiled as C++.
**    It throws exactly one pointer and catches with catch(...) -- no
**    type matching, no destructors -- so instead of a libc++abi built
**    with wasm EH (which no distro ships), the micro-runtime below
**    provides the five __cxa entry points and dummy typeinfo vtables
**    that catch(...) never inspects. Build with -fwasm-exceptions.
**
**    C (the constitutional preference, blocked today): clang's
**    -mllvm -wasm-enable-sjlj pass rewrites setjmp/longjmp onto wasm
**    EH and expects the three __wasm_setjmp* helpers below (the same
**    runtime current wasi-libc ships; vendored so any wasi-libc
**    works, header in wasi/setjmp.h). As of this writing every
**    LLVM available to us miscompiles this path: 18 segfaults, 19.1.1
**    emits structurally invalid catch placement, 21 rejects its own
**    tag symbols. Kept for the day the C path heals.
**
** 2. Hosted-libc corners WASI leaves out or gets wrong. What can be
**    honest is implemented (tmpnam/tmpfile against the preopened
**    directory; fopen("") failing with ENOENT; setlocale admitting
**    only the C locale exists); what cannot (system -- there are no
**    processes) reports failure, which os.execute() surfaces at the
**    Lua level exactly as on any platform without a shell.
*/
#if defined(__wasm__)

#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>

#ifndef L_tmpnam        /* wasi stdio.h declares tmpnam but not L_tmpnam */
#define L_tmpnam 32
#endif

#if defined(__cplusplus)
#ifndef LUAW_EXTERNAL_EH
/* LUAW_EXTERNAL_EH: define to suppress this micro-runtime and let a real
   C++ ABI library (a libc++abi built with -fwasm-exceptions) own exception
   dispatch. An embedder whose host code needs typed catches -- not just
   Lua's own catch(...) -- must use that mode, so Lua errors and the host's
   typed exceptions travel one coherent EH domain. See the Makefile's
   WASM_EH knob, which wires this define together with the external libc++abi
   link line. Default (undefined) keeps the self-contained shim below. */

extern "C" {

struct __luawasm_eh_header {
  void *typeinfo;
  void (*dtor)(void *);
};

void *__cxa_allocate_exception (size_t size) {
  char *p = (char *)malloc(sizeof(struct __luawasm_eh_header) + size);
  return p + sizeof(struct __luawasm_eh_header);
}

void __cxa_free_exception (void *obj) {
  free((char *)obj - sizeof(struct __luawasm_eh_header));
}

void __cxa_throw (void *obj, void *tinfo, void (*dtor)(void *)) {
  struct __luawasm_eh_header *h =
    (struct __luawasm_eh_header *)((char *)obj - sizeof(struct __luawasm_eh_header));
  h->typeinfo = tinfo;
  h->dtor = dtor;
  __builtin_wasm_throw(0, obj);  /* tag 0: __cpp_exception */
}

void *__cxa_begin_catch (void *obj) { return obj; }

void __cxa_end_catch (void) {}

void __gxx_wasm_personality_v0 (void) {}

/* catch(...) never inspects type_info; these exist only as symbols for
   the throw site's weak typeinfo objects */
char __luawasm_ptr_ti_vtable[32] __asm__("_ZTVN10__cxxabiv119__pointer_type_infoE");
char __luawasm_class_ti_vtable[32] __asm__("_ZTVN10__cxxabiv117__class_type_infoE");

void __luawasm_terminate (void) __asm__("_ZSt9terminatev");
void __luawasm_terminate (void) { abort(); }

}  /* extern "C" */

#endif /* !LUAW_EXTERNAL_EH */

#else /* !__cplusplus: the C sjlj runtime */

#include "wasi/setjmp.h"

struct __WasmLongjmpArgs {
  void *env;
  int val;
};

void __wasm_setjmp (void *env, uint32_t label, void *func_invocation_id) {
  struct __wasm_jmp_buf_tag *jb = env;
  jb->func_invocation_id = func_invocation_id;
  jb->label = label;
}

uint32_t __wasm_setjmp_test (void *env, void *func_invocation_id) {
  struct __wasm_jmp_buf_tag *jb = env;
  return (jb->func_invocation_id == func_invocation_id) ? jb->label : 0;
}

_Noreturn void __wasm_longjmp (void *env, int val) {
  struct __wasm_jmp_buf_tag *jb = env;
  if (val == 0) val = 1;
  jb->arg.env = env;
  jb->arg.val = val;
  __builtin_wasm_throw(1, &jb->arg);  /* tag 1: __c_longjmp */
}

#endif /* __cplusplus */

#ifdef __cplusplus
extern "C" {
#endif

int system (const char *cmd) {
  (void)cmd;
  errno = ENOTSUP;
  return cmd ? -1 : 0;  /* no shell available */
}

/* no /tmp in the guest namespace; hand out unused names in the
   working directory instead (checked with access, like tmpnam) */
char *tmpnam (char *s) {
  static char buf[L_tmpnam];
  static unsigned counter = 0;
  char *out = (s != NULL) ? s : buf;
  unsigned tries;
  for (tries = 0; tries < 10000; tries++) {
    snprintf(out, L_tmpnam, "lua_tmp_%u", counter++);
    if (access(out, F_OK) != 0)
      return out;
  }
  return NULL;
}

/* an anonymous scratch file in the working directory: created under a
   fresh tmpnam name, then unlinked while open (POSIX tmpfile
   semantics; on hosts that refuse to unlink open files the name
   simply persists until close) */
FILE *tmpfile (void) {
  char name[L_tmpnam];
  FILE *f;
  if (tmpnam(name) == NULL)
    return NULL;
  f = fopen(name, "w+b");
  if (f != NULL)
    remove(name);
  return f;
}

/* wasi path resolution opens "" as the preopened directory itself;
   POSIX says ENOENT. Reimplemented over open()/fdopen() so the empty
   path fails honestly (everything else passes straight through) */
FILE *fopen (const char *name, const char *mode) {
  int flags;
  if (name == NULL || name[0] == '\0') {
    errno = ENOENT;
    return NULL;
  }
  switch (mode[0]) {
    case 'r': flags = O_RDONLY; break;
    case 'w': flags = O_WRONLY | O_CREAT | O_TRUNC; break;
    case 'a': flags = O_WRONLY | O_CREAT | O_APPEND; break;
    default: errno = EINVAL; return NULL;
  }
  if (strchr(mode, '+'))
    flags = (flags & ~(O_RDONLY | O_WRONLY)) | O_RDWR;
  int fd = open(name, flags, 0666);
  if (fd < 0)
    return NULL;
  FILE *f = fdopen(fd, mode);
  if (f == NULL)
    close(fd);
  return f;
}

/* wasi-libc's setlocale claims success for any locale name while only
   the C locale actually exists; report reality instead so locale-
   guarded code (os.setlocale users) takes its no-locale path */
char *setlocale (int category, const char *locale) {
  (void)category;
  if (locale == NULL || locale[0] == '\0' ||
      strcmp(locale, "C") == 0 || strcmp(locale, "POSIX") == 0)
    return (char *)"C";
  return NULL;
}

#ifdef __cplusplus
}  /* extern "C" */
#endif


#endif /* __wasm__ */

/* When compiled as C++, keep C linkage throughout: a C downstream that
** links this in (examples/embed) calls back into the VM, and the
** boundary must agree on symbol names. Linkage only -- inside these
** braces the code still compiles as C++ (which is how LUAI_THROW
** becomes a real throw). */
#if defined(__cplusplus)
extern "C" {
#endif

/* core */
#include "lzio.c"
#include "lctype.c"
#include "lopcodes.c"
#include "lmem.c"
#include "lundump.c"
#include "ldump.c"
#include "lstate.c"
#include "lgc.c"
#include "llex.c"
#undef next  /* llex.c's file-local macro; hostile to libstdc++ headers
                that C++ hosted builds may pull in after this point */
#include "lcode.c"
#include "lparser.c"
#include "ldebug.c"
#include "lfunc.c"
#include "lobject.c"
#include "ltm.c"
#include "lstring.c"
#include "ltable.c"
#include "ldo.c"
#include "lvm.c"
#include "lapi.c"

/* auxiliary library */
#include "lauxlib.c"

/* standard libraries */
#include "lbaselib.c"
#include "lcorolib.c"
#include "ldblib.c"
#include "liolib.c"
#include "lmathlib.c"
#include "loadlib.c"
#include "loslib.c"
#include "lstrlib.c"
#include "ltablib.c"
#include "lutf8lib.c"
#include "linit.c"

/* The witness debug build (-DLUA_LTESTS): vendored upstream ltests
** (tests/ltests/) instruments the VM -- checked allocator, internal
** assertions, and the T library that unlocks the suite's C-API
** battery. ltests.c compiles as its own translation unit (it clashes
** with lstrlib.c's namespace inside one TU), so build with
**   -DLUA_LTESTS '-DLUA_USER_H="ltests.h"' -Itests/ltests -Isrc \
**   src/onelua.c tests/ltests/ltests.c
** Witness-only; never the shipped artifact.
**
** ltests.h gates its own luaL_newstate/luaL_openlibs wrappers on
** lua.c's 'lua_c' macro, which an amalgamation cannot set TU-wide
** without clobbering lauxlib.c's real definitions -- so the same two
** wrappers are applied here, scoped to the interpreter below. */
#if defined(LUA_LTESTS)
static lua_State *luaw_ltests_newstate (void) {
  return lua_newstate(debug_realloc, &l_memcontrol);
}
static void luaw_ltests_openlibs (lua_State *L) {
  luaL_openlibs(L);
  luaL_requiref(L, "T", luaB_opentests, 1);
  lua_pop(L, 1);
}
#define luaL_newstate	luaw_ltests_newstate
#define luaL_openlibs	luaw_ltests_openlibs
#endif

/* interpreter */
#ifdef MAKE_LUA
#include "lua.c"
#endif

/*
** ── The embedding interface ─────────────────────────────────────────
** MAKE_REACTOR wasm builds are reactors: the host calls in, the VM
** returns. The host interface is WASI (stdio, filesystem, clock --
** print() arrives at the host as fd_write on fd 1) plus the exports
** below, and nothing else.
**
** This glue is opt-in (-DMAKE_REACTOR, set by the Makefile's wasm-lib
** target), NOT implied by MAKE_LIB. A downstream that links Lua into
** its own wasm32-wasi artifact (issue #11) compiles onelua.c with plain
** -DMAKE_LIB and drives the VM through lua.h directly; it must not
** inherit these luaw_* exports, whose reactor coroutine model is a
** property of *this* project's finished artifact, not of the embeddable
** core. So the reactor belongs to the artifact build alone.
**
** The contract, per the constitution: no call here ever blocks. A
** long-lived program (a game's main loop) runs as a coroutine --
** luaw_start loads it, and the host pumps luaw_step once per frame;
** each yield returns control to the host. No threads, no
** SharedArrayBuffer, no Asyncify, ever.
**
** Errors never unwind into the host: every entry point traps them and
** returns a Lua status code (LUA_OK == 0); the message is held in the
** registry (so the pointer stays valid) until the next entry call.
*/
#if defined(__wasm__) && defined(MAKE_REACTOR)

#define LUAW_API __attribute__((used, visibility("default")))

static lua_State *luaw_L = NULL;

static const char *const LUAW_ERRKEY = "luaw.error";
static const char *const LUAW_COKEY  = "luaw.coroutine";

/* stash the error at the top of 'from's stack; keep it alive in the
   registry of the main state so the returned pointer stays valid */
static void luaw_seterror (lua_State *from) {
  const char *msg = lua_tostring(from, -1);
  lua_pushstring(luaw_L, msg ? msg : "(error object is not a string)");
  lua_setfield(luaw_L, LUA_REGISTRYINDEX, LUAW_ERRKEY);
  lua_pop(from, 1);
}

static void luaw_clearerror (void) {
  lua_pushnil(luaw_L);
  lua_setfield(luaw_L, LUA_REGISTRYINDEX, LUAW_ERRKEY);
}

/* create the VM: standard libraries; returns 0 on success */
LUAW_API int luaw_init (void) {
  if (luaw_L != NULL) return 0;
  luaw_L = luaL_newstate();
  if (luaw_L == NULL) return LUA_ERRMEM;
  luaL_openlibs(luaw_L);
  return 0;
}

/* buffer management for passing chunks in from the host */
LUAW_API void *luaw_alloc (size_t n) { return malloc(n); }
LUAW_API void luaw_free (void *p) { free(p); }

/* the last error message (NUL-terminated), or NULL if the last entry
   call succeeded; valid until the next entry call */
LUAW_API const char *luaw_last_error (void) {
  const char *msg;
  if (luaw_L == NULL) return "luaw_init has not been called";
  lua_getfield(luaw_L, LUA_REGISTRYINDEX, LUAW_ERRKEY);
  msg = lua_tostring(luaw_L, -1);
  lua_pop(luaw_L, 1);
  return msg;   /* string stays alive via the registry reference */
}

/* load and run a chunk to completion; returns a Lua status code */
LUAW_API int luaw_run (const char *chunk, size_t len, const char *name) {
  int status;
  if (luaw_L == NULL) return LUA_ERRRUN;
  luaw_clearerror();
  status = luaL_loadbuffer(luaw_L, chunk, len, name ? name : "=(embedded)");
  if (status == LUA_OK)
    status = lua_pcall(luaw_L, 0, 0, 0);
  if (status != LUA_OK)
    luaw_seterror(luaw_L);
  return status;
}

/* load a chunk as the resident program (a coroutine); the host then
   pumps luaw_step. Replaces any previous resident program. */
LUAW_API int luaw_start (const char *chunk, size_t len, const char *name) {
  int status;
  lua_State *co;
  if (luaw_L == NULL) return LUA_ERRRUN;
  luaw_clearerror();
  co = lua_newthread(luaw_L);
  status = luaL_loadbuffer(co, chunk, len, name ? name : "=(program)");
  if (status != LUA_OK) {
    luaw_seterror(co);
    lua_pop(luaw_L, 1);  /* drop the thread */
    return status;
  }
  lua_setfield(luaw_L, LUA_REGISTRYINDEX, LUAW_COKEY);
  return LUA_OK;
}

/* resume the resident program for one step. LUA_YIELD: it yielded and
   is still alive -- call again next frame. LUA_OK: it finished. Any
   other status: it failed; the message is in luaw_last_error(). */
LUAW_API int luaw_step (void) {
  int status, nres;
  lua_State *co;
  if (luaw_L == NULL) return LUA_ERRRUN;
  luaw_clearerror();
  lua_getfield(luaw_L, LUA_REGISTRYINDEX, LUAW_COKEY);
  co = lua_tothread(luaw_L, -1);
  lua_pop(luaw_L, 1);
  if (co == NULL) {
    lua_pushliteral(luaw_L, "no resident program (luaw_start first)");
    lua_setfield(luaw_L, LUA_REGISTRYINDEX, LUAW_ERRKEY);
    return LUA_ERRRUN;
  }
  status = lua_resume(co, luaw_L, 0, &nres);
  if (status == LUA_YIELD) {
    lua_pop(co, nres);   /* yield values are the host's business only
                            through WASI for now; keep the contract small */
    return LUA_YIELD;
  }
  if (status != LUA_OK)
    luaw_seterror(co);
  /* finished or failed either way: release the program */
  lua_pushnil(luaw_L);
  lua_setfield(luaw_L, LUA_REGISTRYINDEX, LUAW_COKEY);
  return status;
}

#endif /* __wasm__ && MAKE_REACTOR */

#if defined(__cplusplus)
}  /* extern "C" */
#endif
