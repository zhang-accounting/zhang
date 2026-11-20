#ifndef extism_pdk_h
#define extism_pdk_h

#ifdef __cplusplus
extern "C" {
#endif

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef uint64_t ExtismHandle;

#define EXTISM_ENV_MODULE "extism:host/env"
#define EXTISM_USER_MODULE "extism:host/user"

#define EXTISM_EXPORT_AS(name) __attribute__((export_name(name)))
#define EXTISM_EXPORTED_FUNCTION(name)                                         \
  EXTISM_EXPORT_AS(#name)                                                      \
  name(void)

#define EXTISM_IMPORT(a, b) __attribute__((import_module(a), import_name(b)))
#define EXTISM_IMPORT_ENV(b)                                                   \
  __attribute__((import_module(EXTISM_ENV_MODULE), import_name(b)))
#define EXTISM_IMPORT_USER(b)                                                  \
  __attribute__((import_module(EXTISM_USER_MODULE), import_name(b)))

EXTISM_IMPORT_ENV("length_unsafe")
extern uint64_t extism_length(const ExtismHandle);
EXTISM_IMPORT_ENV("alloc")
extern ExtismHandle extism_alloc(const uint64_t);
EXTISM_IMPORT_ENV("free")
extern void extism_free(ExtismHandle);

EXTISM_IMPORT_ENV("error_set")
extern void extism_error_set(const ExtismHandle);

EXTISM_IMPORT_ENV("config_get")
extern ExtismHandle extism_config_get(const ExtismHandle);

EXTISM_IMPORT_ENV("var_get")
extern ExtismHandle extism_var_get(const ExtismHandle);

EXTISM_IMPORT_ENV("var_set")
extern void extism_var_set(ExtismHandle, const ExtismHandle);

EXTISM_IMPORT_ENV("http_request")
extern ExtismHandle extism_http_request(const ExtismHandle, const ExtismHandle);

EXTISM_IMPORT_ENV("http_status_code")
extern int32_t extism_http_status_code(void);

EXTISM_IMPORT_ENV("http_headers")
extern ExtismHandle extism_http_headers(void);

EXTISM_IMPORT_ENV("log_info")
extern void extism_log_info(const ExtismHandle);
EXTISM_IMPORT_ENV("log_debug")
extern void extism_log_debug(const ExtismHandle);
EXTISM_IMPORT_ENV("log_warn")
extern void extism_log_warn(const ExtismHandle);
EXTISM_IMPORT_ENV("log_error")
extern void extism_log_error(const ExtismHandle);
EXTISM_IMPORT_ENV("log_trace")
extern void extism_log_trace(const ExtismHandle);
EXTISM_IMPORT_ENV("get_log_level")
extern int32_t extism_get_log_level();

EXTISM_IMPORT_ENV("input_offset")
extern ExtismHandle extism_input_offset(void);

EXTISM_IMPORT_ENV("input_length")
extern uint64_t extism_input_length(void);
// extism_length(extism_input_offset()); is also valid

// Load data from Extism memory, verifies load is inbounds
bool extism_load_from_handle(const ExtismHandle src, const uint64_t src_offset,
                             void *dest, const size_t n);

// Load data from input buffer, verifies load is inbounds
static inline bool extism_load_input(const uint64_t src_offset, void *dest,
                                     const size_t n) {
  return extism_load_from_handle(extism_input_offset(), src_offset, dest, n);
}

// Load n-1 bytes and zero terminate
// Verifies load is inbounds
bool extism_load_sz(const ExtismHandle src, uint64_t src_offset, char *dest,
                    const size_t n);

// Load n-1 bytes from input buffer and zero terminate
// Verifies load is inbounds
static inline bool extism_load_input_sz(const uint64_t src_offset, char *dest,
                                        const size_t n) {
  return extism_load_sz(extism_input_offset(), src_offset, dest, n);
}

// Copy data into Extism memory, verifies store is in bounds
bool extism_store_to_handle(ExtismHandle dest, const uint64_t dest_offset,
                            const void *buffer, const size_t n);

// Allocate a buffer in Extism memory and copy into it
ExtismHandle extism_alloc_buf(const void *src, const size_t n);

__attribute__((
    deprecated("Use extism_alloc_buf instead."))) static inline ExtismHandle
extism_alloc_string(const char *s, const size_t n) {
  return extism_alloc_buf(s, n);
}

#ifdef EXTISM_USE_LIBC
// get the length (n) and malloc(n), load n bytes from Extism memory
// into it. If outSize is provided, set it to n
void *extism_load_dup(const ExtismHandle h, size_t *outSize);

// get the input length (n) and malloc(n), load n bytes from Extism memory
// into it. If outSize is provided, set it to n
static inline void *extism_load_input_dup(size_t *outSize) {
  return extism_load_dup(extism_input_offset(), outSize);
}

// get the length, add 1 to it to get n. malloc(n), load n - 1 bytes
// from Extism memory into it. Zero terminate. If outSize is provided, set it
// to n
char *extism_load_sz_dup(const ExtismHandle h, size_t *outSize);

// get the input length, add 1 to it to get n. malloc(n), load n - 1 bytes
// from Extism memory into it. Zero terminate. If outSize is provided, set it
// to n
static inline char *extism_load_input_sz_dup(size_t *outSize) {
  return extism_load_sz_dup(extism_input_offset(), outSize);
}

#endif // EXTISM_USE_LIBC

// Allocate a buffer in Extism memory and copy string data into it
// copied string is NOT null terminated
ExtismHandle extism_alloc_buf_from_sz(const char *sz);

typedef enum {
  ExtismLogTrace,
  ExtismLogDebug,
  ExtismLogInfo,
  ExtismLogWarn,
  ExtismLogError,
} ExtismLog;

// Write to Extism log
void extism_log(const char *s, const size_t len, const ExtismLog level);

// Write zero-terminated string to Extism log
void extism_log_sz(const char *s, const ExtismLog level);

// Set the output from an ExtismHandle, returns false if outside the memory
// block is specified.
bool extism_output_set_from_handle(const ExtismHandle handle,
                                   const uint64_t offset, const uint64_t n);

// Set the output to the entire contents of an ExtismHandle
void extism_output_handle(const ExtismHandle handle);

// Alloc a buf of Extism memory and output it
void extism_output_buf(const void *src, const size_t n);

// set output to extism_alloc_buf_from_sz
void extism_output_buf_from_sz(const char *sz);

// output an error from a buf
void extism_error_set_buf(const char *message, const size_t messageLen);

// output an error from a sz
void extism_error_set_buf_from_sz(const char *message);

// get a config var from a buf key
ExtismHandle extism_config_get_buf(const char *name, const size_t nameLen);

// get a config var from a sz key
ExtismHandle extism_config_get_buf_from_sz(const char *name);

// get a var from a buf key
ExtismHandle extism_var_get_buf(const char *name, const size_t nameLen);

// get a var from a sz key
ExtismHandle extism_var_get_buf_from_sz(const char *name);

// store a var from a buf key
void extism_var_set_buf(const char *name, const size_t nameLen,
                        const ExtismHandle value);

// store a var from a sz key
void extism_var_set_buf_from_sz(const char *name, const ExtismHandle value);

#ifdef __cplusplus
}
#endif
#endif // extism_pdk_h

// avoid greying out the implementation section
#if defined(Q_CREATOR_RUN) || defined(__INTELLISENSE__) ||                     \
    defined(_CDT_PARSER__)
#define EXTISM_IMPLEMENTATION
#endif

#ifndef extism_low_level_imports_h
#if defined(EXTISM_ENABLE_LOW_LEVEL_API) || defined(EXTISM_IMPLEMENTATION)
#define extism_low_level_imports_h

#ifdef __cplusplus
extern "C" {
#endif

#ifdef EXTISM_ENABLE_LOW_LEVEL_API
typedef uint64_t ExtismPointer;
#else
#define ExtismPointer uint64_t
#endif

EXTISM_IMPORT_ENV("input_load_u8")
extern uint8_t __extism_input_load_u8(const uint64_t);
EXTISM_IMPORT_ENV("input_load_u64")
extern uint64_t __extism_input_load_u64(const uint64_t);
EXTISM_IMPORT_ENV("output_set")
extern void __extism_output_set(const ExtismPointer, const uint64_t);
EXTISM_IMPORT_ENV("store_u8")
extern void __extism_store_u8(ExtismPointer, const uint8_t);
EXTISM_IMPORT_ENV("load_u8")
extern uint8_t __extism_load_u8(const ExtismPointer);
EXTISM_IMPORT_ENV("store_u64")
extern void __extism_store_u64(ExtismPointer, const uint64_t);
EXTISM_IMPORT_ENV("load_u64")
extern uint64_t __extism_load_u64(const ExtismPointer);

#ifdef EXTISM_ENABLE_LOW_LEVEL_API
#define extism_input_load_u8 __extism_input_load_u8
#define extism_input_load_u64 __extism_input_load_u64
#define extism_output_set __extism_output_set
#define extism_store_u8 __extism_store_u8
#define extism_load_u8 __extism_load_u8
#define extism_store_u64 __extism_store_u64
#define extism_load_u64 __extism_load_u64

// Load data from Extism memory, does not verify load is in bounds
#define extism_load __extism_load

// Copy data into Extism memory, does not verify store is inbounds
#define extism_store __extism_store

// Load n-1 bytes and zero terminate
// Does not verify load is inbounds
void extism_load_sz_unsafe(const ExtismPointer src, char *dest, const size_t n);

// Returns 0 when the pointer doesn't refer to the start of
// the data section of a memory block.
EXTISM_IMPORT_ENV("length")
extern uint64_t extism_length_safe(const ExtismPointer);

#else
#undef ExtismPointer
#endif

#ifdef __cplusplus
}
#endif
#endif // defined(EXTISM_ENABLE_LOW_LEVEL_API) || defined(EXTISM_IMPLEMENTATION)
#endif // extism_low_level_imports_h

#ifdef EXTISM_IMPLEMENTATION
#ifndef extism_pdk_c
#define extism_pdk_c

#ifndef EXTISM_ENABLE_LOW_LEVEL_API
#define ExtismPointer uint64_t
#define EXTISM_LL_API static
#else
#define EXTISM_LL_API
#endif

// Load data from Extism memory, does not verify load is in bounds
EXTISM_LL_API void __extism_load(const ExtismPointer offs, void *dest,
                                 const size_t n) {
  const size_t chunk_count = n >> 3;
  uint64_t *i64_buffer = dest;
  for (size_t chunk_idx = 0; chunk_idx < chunk_count; chunk_idx++) {
    i64_buffer[chunk_idx] = __extism_load_u64(offs + (chunk_idx << 3));
  }

  size_t remainder_offset = chunk_count << 3;
  const size_t remainder_end = remainder_offset + (n & 7);
  for (uint8_t *u8_buffer = dest; remainder_offset < remainder_end;
       remainder_offset++) {
    u8_buffer[remainder_offset] = __extism_load_u8(offs + remainder_offset);
  }
}

bool extism_load_from_handle(const ExtismHandle src, const uint64_t src_offset,
                             void *dest, const size_t n) {
  if ((src_offset + n) > extism_length(src)) {
    return false;
  }
  const ExtismPointer offs = src + src_offset;
  __extism_load(offs, dest, n);
  return true;
}

// Load n-1 bytes and zero terminate
// Does not verify load is inbounds
EXTISM_LL_API void extism_load_sz_unsafe(const ExtismPointer src, char *dest,
                                         const size_t n) {
  __extism_load(src, dest, n - 1);
  dest[n - 1] = '\0';
}

// Load n-1 bytes and zero terminate
// Verifies load is inbounds
bool extism_load_sz(const ExtismHandle src, uint64_t src_offset, char *dest,
                    const size_t n) {
  const uint64_t len = extism_length(src);
  if ((src_offset + n - 1) > len) {
    return false;
  }
  extism_load_sz_unsafe(src + src_offset, dest, n);
  return true;
}

// Copy data into Extism memory, does not verify store is inbounds
EXTISM_LL_API void __extism_store(ExtismPointer offs, const void *buffer,
                                  const size_t length) {
  const size_t chunk_count = length >> 3;
  const uint64_t *i64_buffer = buffer;
  for (size_t chunk_idx = 0; chunk_idx < chunk_count; chunk_idx++) {
    __extism_store_u64(offs + (chunk_idx << 3), i64_buffer[chunk_idx]);
  }

  size_t remainder_offset = chunk_count << 3;
  const size_t remainder_end = remainder_offset + (length & 7);
  for (const uint8_t *u8_buffer = buffer; remainder_offset < remainder_end;
       remainder_offset++) {
    __extism_store_u8(offs + remainder_offset, u8_buffer[remainder_offset]);
  }
}

// Copy data into Extism memory, verifies store is in bounds
bool extism_store_to_handle(ExtismHandle dest, const uint64_t dest_offset,
                            const void *buffer, const size_t n) {
  if ((dest_offset + n) > extism_length(dest)) {
    return false;
  }
  ExtismPointer offs = dest + dest_offset;
  __extism_store(offs, buffer, n);
  return true;
}

// Allocate a buffer in Extism memory and copy into it
ExtismHandle extism_alloc_buf(const void *src, const size_t n) {
  ExtismHandle ptr = extism_alloc(n);
  __extism_store(ptr, src, n);
  return ptr;
}

#ifdef EXTISM_USE_LIBC
#include <stdlib.h>
#include <string.h>

#define extism_strlen strlen

// get the length (n) and malloc(n), load n bytes from Extism memory
// into it. If outSize is provided, set it to n
void *extism_load_dup(const ExtismHandle h, size_t *outSize) {
  const uint64_t n = extism_length(h);
  if (n > SIZE_MAX) {
    return NULL;
  }
  void *buf = malloc(n);
  if (!buf) {
    return NULL;
  }
  __extism_load(h, buf, n);
  if (outSize) {
    *outSize = n;
  }
  return buf;
}

// get the length, add 1 to it to get n. malloc(n), load n - 1 bytes
// from Extism memory into it. Zero terminate. If outSize is provided, set it
// to n
char *extism_load_sz_dup(const ExtismHandle h, size_t *outSize) {
  uint64_t n = extism_length(h);
  if (n > (SIZE_MAX - 1)) {
    return NULL;
  }
  n++;
  char *buf = malloc(n);
  if (!buf) {
    return NULL;
  }
  extism_load_sz_unsafe(h, buf, n);
  if (outSize) {
    *outSize = n;
  }
  return buf;
}

#else
static size_t extism_strlen(const char *sz) {
  size_t len;
  for (len = 0; sz[len] != '\0'; len++) {
  }
  return len;
}
#endif

// Allocate a buffer in Extism memory and copy string data into it
// copied string is NOT null terminated
ExtismHandle extism_alloc_buf_from_sz(const char *sz) {
  return extism_alloc_buf(sz, extism_strlen(sz));
}

// Write to Extism log
void extism_log(const char *s, const size_t len, const ExtismLog level) {
  if (level < extism_get_log_level()) {
    return;
  }

  ExtismHandle buf = extism_alloc(len);
  __extism_store(buf, s, len);
  switch (level) {
  case ExtismLogInfo:
    extism_log_info(buf);
    break;
  case ExtismLogDebug:
    extism_log_debug(buf);
    break;
  case ExtismLogWarn:
    extism_log_warn(buf);
    break;
  case ExtismLogError:
    extism_log_error(buf);
    break;
  case ExtismLogTrace:
    extism_log_trace(buf);
    break;
  }
}

// Write zero-terminated string to Extism log
void extism_log_sz(const char *s, const ExtismLog level) {
  const size_t len = extism_strlen(s);
  extism_log(s, len, level);
}

// Set the output from an ExtismHandle, returns false if outside the memory
// block is specified.
bool extism_output_set_from_handle(const ExtismHandle handle,
                                   const uint64_t offset, const uint64_t n) {
  if ((offset + n) > extism_length(handle)) {
    return false;
  }
  __extism_output_set(handle + offset, n);
  return true;
}

// Set the output to the entire contents of an ExtismHandle
void extism_output_handle(const ExtismHandle handle) {
  __extism_output_set(handle, extism_length(handle));
}

// Alloc a buf of Extism memory and output it
void extism_output_buf(const void *src, const size_t n) {
  ExtismHandle handle = extism_alloc_buf(src, n);
  __extism_output_set(handle, n);
}

// set output to extism_alloc_buf_from_sz
void extism_output_buf_from_sz(const char *sz) {
  const size_t n = extism_strlen(sz);
  extism_output_buf(sz, n);
}

// output an error from a buf
void extism_error_set_buf(const char *message, const size_t messageLen) {
  ExtismHandle handle = extism_alloc_buf(message, messageLen);
  extism_error_set(handle);
}

// output an error from a sz
void extism_error_set_buf_from_sz(const char *message) {
  const size_t len = extism_strlen(message);
  extism_error_set_buf(message, len);
}

// get a config var from a buf key
ExtismHandle extism_config_get_buf(const char *name, const size_t nameLen) {
  ExtismHandle key = extism_alloc_buf(name, nameLen);
  ExtismHandle value = extism_config_get(key);
  return value;
}

// get a config var from a sz key
ExtismHandle extism_config_get_buf_from_sz(const char *name) {
  const size_t len = extism_strlen(name);
  return extism_config_get_buf(name, len);
}

// get a var from a buf key
ExtismHandle extism_var_get_buf(const char *name, const size_t nameLen) {
  ExtismHandle key = extism_alloc_buf(name, nameLen);
  ExtismHandle value = extism_var_get(key);
  return value;
}

// get a var from a sz key
ExtismHandle extism_var_get_buf_from_sz(const char *name) {
  const size_t len = extism_strlen(name);
  return extism_var_get_buf(name, len);
}

// store a var from a buf key
void extism_var_set_buf(const char *name, const size_t nameLen,
                        const ExtismHandle value) {
  ExtismHandle key = extism_alloc_buf(name, nameLen);
  extism_var_set(key, value);
}

// store a var from a sz key
void extism_var_set_buf_from_sz(const char *name, const ExtismHandle value) {
  const size_t len = extism_strlen(name);
  extism_var_set_buf(name, len, value);
}

#ifndef EXTISM_ENABLE_LOW_LEVEL_API
#undef ExtismPointer
#endif

#endif // extism_pdk_c
#endif // EXTISM_IMPLEMENTATION
