/*
** <setjmp.h> for wasm32-wasi builds. The wasi-libc snapshot this repo
** builds against predates setjmp support, so the header is vendored.
**
** C builds: clang's sjlj lowering (-mllvm -wasm-enable-sjlj) intercepts
** every setjmp/longjmp call, so no library implementation of these two
** functions exists anywhere -- only the __wasm_setjmp* runtime in
** onelua.c, whose state lives in the jmp_buf defined here. Layout
** matches wasi-libc's own.
**
** C++ builds never call setjmp (Lua's error handling becomes
** throw/catch), but ldo.c includes <setjmp.h> unconditionally, so the
** declarations must still parse.
*/
#ifndef _SETJMP_H
#define _SETJMP_H

#if !defined(__wasm__)
#error "this setjmp.h is only for wasm builds"
#endif

#if defined(__cplusplus)
#define _LUAWASM_NORETURN [[noreturn]]
extern "C" {
#else
#define _LUAWASM_NORETURN _Noreturn
#endif

struct __wasm_jmp_buf_tag {
  void *func_invocation_id;
  unsigned int label;
  struct {          /* payload thrown by __wasm_longjmp */
    void *env;
    int val;
  } arg;
};

typedef struct __wasm_jmp_buf_tag jmp_buf[1];

int setjmp (jmp_buf env);
_LUAWASM_NORETURN void longjmp (jmp_buf env, int val);

#if defined(__cplusplus)
}
#endif

#endif
