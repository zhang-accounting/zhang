// The `json` library of every script: a JSON codec over the Lua C API, compiled into the interpreter module with
// runtime.cpp, whose prelude exposes it as `json`. It replaced dkjson (pure Lua, about 3 MB/s) and keeps the rules the
// Lua plugins guide states (docs/src/content/docs/guides/lua-plugins.md, "JSON"):
//
// - decode: RFC 8259 JSON in UTF-8 and nothing after it (no comments, no trailing commas). null is `json.null`, an
//   object a table with the object marker, an array a sequence with the array marker. An integer that fits 64 bits
//   is a Lua integer; any other number is the nearest float, and one past the float range is an error. An error
//   names the line and the column (in bytes) as dkjson's did.
// - encode: nil and `json.null` are null. A table marked as an object (`json.object`, a decoded object) is an object;
//   any other table is an array when its keys are 1..n, holes allowed unless it is sparse (its largest key past 10
//   and more than twice its count), and an object otherwise, whose keys (strings and numbers) are written sorted:
//   numbers first, ascending, then strings by byte, so one value always gives the same bytes. An integer is written
//   exactly; a float with the fewest digits that read back as the same float (".0" when it looks like an integer, as
//   Lua prints it), NaN and the infinities as null. A string must be UTF-8 text; control characters and the code
//   points dkjson escaped are written as \u escapes.
// - limits: 1000 levels of nesting both ways. Both directions loop over an explicit stack instead of recursing, so a
//   deep document grows neither the C nor the wasm stack; every byte they allocate is a Lua object, under the call's
//   memory budget, so a refused allocation is Lua's own memory error.

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
extern "C" {
#include "lua.h"
#include "lauxlib.h"
}

int json_open(lua_State *L);

namespace {

// how deep arrays and objects may nest, decoding and encoding (the messages below and the guide say 1000)
const int MAX_DEPTH = 1000;
static_assert(MAX_DEPTH == 1000, "the error messages and the Lua plugins guide name the limit");

// the upvalues of encode and decode: json.null and the shared metatables marking objects and arrays
const int NULL_VALUE = lua_upvalueindex(1);
const int OBJECT_META = lua_upvalueindex(2);
const int ARRAY_META = lua_upvalueindex(3);

const char HEX[] = "0123456789abcdef";

template <typename... Args>
[[noreturn]] void fail(lua_State *L, const char *format, Args... args) {
  luaL_error(L, format, args...);
  __builtin_unreachable();
}

// The collector is paused while a value is encoded or decoded: no finalizer runs Lua code in the middle (it could
// change the table being written, whose string keys this file holds as pointers), and a document being built is not
// traversed again and again. An allocation the budget refuses still runs Lua's emergency collection, which runs no
// finalizer, before it fails. A Lua error unwinding the scope restarts the collector too (Lua errors are C++
// exceptions in this build, see runtime.cpp's HostBlock).
struct CollectorPause {
  lua_State *L;
  bool running;
  explicit CollectorPause(lua_State *L) : L(L), running(lua_gc(L, LUA_GCISRUNNING) == 1) {
    if (running) lua_gc(L, LUA_GCSTOP);
  }
  CollectorPause(const CollectorPause &) = delete;
  CollectorPause &operator=(const CollectorPause &) = delete;
  ~CollectorPause() {
    if (running) lua_gc(L, LUA_GCRESTART);
  }
};

// A growable byte buffer whose memory is a userdata in a fixed slot of the Lua stack: it counts against the memory
// budget, a growth the budget refuses raises Lua's memory error, and the collector frees it however the call ends.
struct Buffer {
  lua_State *L;
  int slot;
  char *data;
  size_t length;
  size_t capacity;

  Buffer(lua_State *L, size_t initial) : L(L), length(0), capacity(initial) {
    data = (char *)lua_newuserdatauv(L, initial, 0);
    slot = lua_gettop(L);
  }
  Buffer(const Buffer &) = delete;
  Buffer &operator=(const Buffer &) = delete;

  void grow(size_t extra) {
    size_t need = length + extra;
    if (need < length) fail(L, "not enough memory");
    size_t next = capacity;
    while (next < need) next = next > SIZE_MAX / 2 ? need : next * 2;
    char *bigger = (char *)lua_newuserdatauv(L, next, 0);
    memcpy(bigger, data, length);
    lua_replace(L, slot);
    data = bigger;
    capacity = next;
  }
  void reserve(size_t extra) {
    if (capacity - length < extra) grow(extra);
  }
  void put(char c) {
    if (length == capacity) grow(1);
    data[length++] = c;
  }
  void append(const void *bytes, size_t count) {
    reserve(count);
    memcpy(data + length, bytes, count);
    length += count;
  }
};

// The length of the UTF-8 sequence at `p` and its code point, or 0 when the bytes there are not one (RFC 3629: no
// overlong form, no surrogate, nothing past U+10FFFF). It reads no byte past the first that is not a continuation
// byte, so the NUL Lua keeps after every string's last byte stops it at the end.
size_t utf8_sequence(const unsigned char *p, uint32_t *code) {
  unsigned char c = p[0];
  if (c < 0x80) {
    *code = c;
    return 1;
  }
  if (c < 0xC2) return 0;
  if (c < 0xE0) {
    if ((p[1] & 0xC0) != 0x80) return 0;
    *code = ((uint32_t)(c & 0x1F) << 6) | (p[1] & 0x3F);
    return 2;
  }
  if (c < 0xF0) {
    unsigned char low = c == 0xE0 ? 0xA0 : 0x80, high = c == 0xED ? 0x9F : 0xBF;
    if (p[1] < low || p[1] > high || (p[2] & 0xC0) != 0x80) return 0;
    *code = ((uint32_t)(c & 0x0F) << 12) | ((uint32_t)(p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    return 3;
  }
  if (c < 0xF5) {
    unsigned char low = c == 0xF0 ? 0x90 : 0x80, high = c == 0xF4 ? 0x8F : 0xBF;
    if (p[1] < low || p[1] > high || (p[2] & 0xC0) != 0x80 || (p[3] & 0xC0) != 0x80) return 0;
    *code = ((uint32_t)(c & 0x07) << 18) | ((uint32_t)(p[1] & 0x3F) << 12) | ((uint32_t)(p[2] & 0x3F) << 6) |
            (p[3] & 0x3F);
    return 4;
  }
  return 0;
}

// ---- encode -------------------------------------------------------------------------------------------------------

// How a byte below 0x80 is written inside a string: 0 as it is, `u` as \u00XX, any other as a backslash and itself.
// The control characters, the quote, the backslash and DEL are escaped, as dkjson did.
struct AsciiEscapes {
  char of[128];
  constexpr AsciiEscapes() : of() {
    for (int c = 0; c < 0x20; c++) of[c] = 'u';
    of['\b'] = 'b';
    of['\f'] = 'f';
    of['\n'] = 'n';
    of['\r'] = 'r';
    of['\t'] = 't';
    of['"'] = '"';
    of['\\'] = '\\';
    of[0x7F] = 'u';
  }
};
constexpr AsciiEscapes ASCII_ESCAPES;

// the code points past ASCII written as \u escapes, as dkjson wrote them: the C1 controls, the soft hyphen and the
// invisible formatting characters some parsers and browsers mishandle (U+2028 and U+2029 end a line in JavaScript)
bool escaped_code_point(uint32_t c) {
  return (c >= 0x80 && c <= 0x9F) || c == 0xAD || (c >= 0x600 && c <= 0x604) || c == 0x70F || c == 0x17B4 ||
         c == 0x17B5 || (c >= 0x200C && c <= 0x200F) || (c >= 0x2028 && c <= 0x202F) || (c >= 0x2060 && c <= 0x206F) ||
         c == 0xFEFF || (c >= 0xFFF0 && c <= 0xFFFF);
}

void write_unicode_escape(Buffer &out, uint32_t code) {
  char escape[6] = {'\\', 'u', HEX[(code >> 12) & 15], HEX[(code >> 8) & 15], HEX[(code >> 4) & 15], HEX[code & 15]};
  out.append(escape, sizeof escape);
}

void write_string(lua_State *L, Buffer &out, const char *text, size_t length) {
  const unsigned char *start = (const unsigned char *)text, *end = start + length, *p = start, *run = start;
  out.reserve(length + 2);
  out.put('"');
  while (p < end) {
    unsigned char c = *p;
    if (c < 0x80) {
      char escape = ASCII_ESCAPES.of[c];
      if (!escape) {
        p++;
        continue;
      }
      out.append(run, p - run);
      if (escape == 'u') {
        write_unicode_escape(out, c);
      } else {
        char pair[2] = {'\\', escape};
        out.append(pair, 2);
      }
      run = ++p;
      continue;
    }
    uint32_t code;
    size_t count = utf8_sequence(p, &code);
    if (!count) {
      fail(L, "cannot encode a string that is not UTF-8 text: the byte at offset %I is not UTF-8 (send other bytes as a `body`)",
           (lua_Integer)(p - start));
    }
    if (escaped_code_point(code)) {
      out.append(run, p - run);
      write_unicode_escape(out, code);
      run = p + count;
    }
    p += count;
  }
  out.append(run, end - run);
  out.put('"');
}

// an integer in decimal; `text` holds at least 21 bytes
size_t format_integer(char *text, lua_Integer value) {
  char digits[20];
  unsigned long long magnitude = value < 0 ? 0ULL - (unsigned long long)value : (unsigned long long)value;
  size_t count = 0;
  do {
    digits[count++] = (char)('0' + magnitude % 10);
    magnitude /= 10;
  } while (magnitude);
  size_t length = 0;
  if (value < 0) text[length++] = '-';
  while (count) text[length++] = digits[--count];
  return length;
}

// a finite float with the fewest significant digits (15, 16 or 17) that read back as the same float, and ".0" when the
// digits look like an integer, as Lua's tostring writes it (1.0, 1e+20). 15 digits give what Lua's 14 gave whenever
// those 14 were exact. `text` holds at least 32 bytes
size_t format_float(char *text, double value) {
  int length = 0;
  for (int precision = 15; precision <= 17; precision++) {
    length = snprintf(text, 32, "%.*g", precision, value);
    if (strtod(text, NULL) == value) break;
  }
  if (text[strspn(text, "-0123456789")] == '\0') {
    text[length++] = '.';
    text[length++] = '0';
  }
  return (size_t)length;
}

void write_number(lua_State *L, Buffer &out, int index) {
  char text[32];
  size_t length;
  if (lua_isinteger(L, index)) {
    length = format_integer(text, lua_tointeger(L, index));
  } else {
    double value = lua_tonumber(L, index);
    // JSON has no NaN and no infinity; null is what dkjson, JavaScript and serde_json write for them
    if (!isfinite(value)) {
      out.append("null", 4);
      return;
    }
    length = format_float(text, value);
  }
  out.append(text, length);
}

// a key of an object being written: a string (pointing into the Lua string, which the table keeps alive while the
// collector is paused) or a number
enum KeyKind { KEY_INTEGER, KEY_FLOAT, KEY_STRING };

struct Key {
  KeyKind kind;
  const char *text;
  size_t length;
  lua_Integer integer;
  lua_Number number;
};

// whether the integer `i` sorts before the float `f`. A float key is never an integer in the integer range (Lua keys
// such a float as the integer), so below 2^63 it is not integral and (double)i never equals it, even rounded
int integer_before_float(lua_Integer i, lua_Number f) {
  if (f >= 9223372036854775808.0) return 1;
  if (f < -9223372036854775808.0) return 0;
  return (double)i < f;
}

// numbers first, ascending, then strings by byte: dkjson's order (Lua compares strings with strcoll, which is the
// byte order in WASI's only locale)
int compare_keys(const void *left, const void *right) {
  const Key *a = (const Key *)left, *b = (const Key *)right;
  if ((a->kind == KEY_STRING) != (b->kind == KEY_STRING)) return a->kind == KEY_STRING ? 1 : -1;
  if (a->kind == KEY_STRING) {
    int order = memcmp(a->text, b->text, a->length < b->length ? a->length : b->length);
    if (order) return order;
    return (a->length > b->length) - (a->length < b->length);
  }
  if (a->kind == KEY_INTEGER && b->kind == KEY_INTEGER) return (a->integer > b->integer) - (a->integer < b->integer);
  if (a->kind == KEY_FLOAT && b->kind == KEY_FLOAT) return (a->number > b->number) - (a->number < b->number);
  if (a->kind == KEY_INTEGER) return integer_before_float(a->integer, b->number) ? -1 : 1;
  return integer_before_float(b->integer, a->number) ? 1 : -1;
}

void write_key(lua_State *L, Buffer &out, const Key &key) {
  if (key.kind == KEY_STRING) {
    write_string(L, out, key.text, key.length);
    return;
  }
  // a number key is written as Lua writes the number, quoted
  char text[34];
  size_t length = 1;
  text[0] = '"';
  if (key.kind == KEY_INTEGER) {
    length += format_integer(text + 1, key.integer);
  } else if (isfinite(key.number)) {
    length += format_float(text + 1, key.number);
  } else {
    const char *name = key.number > 0 ? "inf" : "-inf";
    memcpy(text + 1, name, strlen(name));
    length += strlen(name);
  }
  text[length++] = '"';
  out.append(text, length);
}

void push_key(lua_State *L, const Key &key) {
  if (key.kind == KEY_STRING) lua_pushlstring(L, key.text, key.length);
  else if (key.kind == KEY_INTEGER) lua_pushinteger(L, key.integer);
  else lua_pushnumber(L, key.number);
}

// whether the table at `index` carries the object marker: the shared metatable of json.object() and of decoded
// objects, or any metatable whose __jsontype is "object", as dkjson read it
bool marked_object(lua_State *L, int index) {
  if (!lua_getmetatable(L, index)) return false;
  bool marked = false;
  if (lua_rawequal(L, -1, OBJECT_META)) {
    marked = true;
  } else if (!lua_rawequal(L, -1, ARRAY_META)) {
    lua_pushliteral(L, "__jsontype");
    lua_rawget(L, -2);
    size_t length;
    const char *kind = lua_type(L, -1) == LUA_TSTRING ? lua_tolstring(L, -1, &length) : NULL;
    marked = kind && length == 6 && memcmp(kind, "object", 6) == 0;
    lua_pop(L, 1);
  }
  lua_pop(L, 1);
  return marked;
}

// whether the table at `index` is an array, and its last index: every key a positive integer and the table not
// sparse (dkjson's rule: an array whose largest key is past 10 and more than twice its count is an object instead)
bool array_extent(lua_State *L, int index, lua_Integer *last) {
  lua_Integer largest = 0, count = 0;
  lua_pushnil(L);
  while (lua_next(L, index)) {
    lua_pop(L, 1);
    if (!lua_isinteger(L, -1)) {
      lua_pop(L, 1);
      return false;
    }
    lua_Integer key = lua_tointeger(L, -1);
    if (key < 1) {
      lua_pop(L, 1);
      return false;
    }
    if (key > largest) largest = key;
    count++;
  }
  if (largest > 10 && largest > 2 * count) return false;
  *last = largest;
  return true;
}

// one table being written: an array going through the indices first..last (1..n), or an object going through its
// sorted keys, which sit at [first, last) of the key buffer; `next` is the one to write next
struct Frame {
  const void *table;
  int slot;
  bool object;
  lua_Integer first;
  lua_Integer next;
  lua_Integer last;
};

struct Encoder {
  lua_State *L;
  Buffer &out;
  Buffer &keys;
  Frame *frames;
  int depth;

  Key *key_at(lua_Integer at) { return (Key *)keys.data + at; }

  // start writing the table at `index` (its frame is pushed), or write null for json.null (nothing is pushed)
  bool open(int index) {
    if (lua_rawequal(L, index, NULL_VALUE)) {
      out.append("null", 4);
      return false;
    }
    const void *table = lua_topointer(L, index);
    for (int level = 0; level < depth; level++) {
      if (frames[level].table == table) fail(L, "reference cycle");
    }
    if (depth == MAX_DEPTH) fail(L, "cannot encode a value nested deeper than %d levels", MAX_DEPTH);
    luaL_checkstack(L, 4, "nesting too deep");
    Frame &frame = frames[depth++];
    frame.table = table;
    frame.slot = index;
    lua_Integer last = 0;
    frame.object = marked_object(L, index) || !array_extent(L, index, &last);
    if (!frame.object) {
      frame.first = frame.next = 1;
      frame.last = last;
      out.put('[');
      return true;
    }
    lua_Integer first = (lua_Integer)(keys.length / sizeof(Key));
    lua_pushnil(L);
    while (lua_next(L, index)) {
      lua_pop(L, 1);
      Key key = {KEY_STRING, NULL, 0, 0, 0};
      int type = lua_type(L, -1);
      if (type == LUA_TSTRING) {
        key.text = lua_tolstring(L, -1, &key.length);
      } else if (type == LUA_TNUMBER && lua_isinteger(L, -1)) {
        key.kind = KEY_INTEGER;
        key.integer = lua_tointeger(L, -1);
      } else if (type == LUA_TNUMBER) {
        key.kind = KEY_FLOAT;
        key.number = lua_tonumber(L, -1);
      } else {
        fail(L, "type '%s' is not supported as a key by JSON.", luaL_typename(L, -1));
      }
      keys.append(&key, sizeof key);
    }
    lua_Integer count = (lua_Integer)(keys.length / sizeof(Key)) - first;
    qsort(key_at(first), (size_t)count, sizeof(Key), compare_keys);
    frame.first = frame.next = first;
    frame.last = first + count;
    out.put('{');
    return true;
  }

  // push the next value of the innermost table, closing the tables that are done; false when the outermost is done
  bool advance() {
    while (depth > 0) {
      Frame &frame = frames[depth - 1];
      if (frame.next < frame.last + (frame.object ? 0 : 1)) {
        if (frame.next > frame.first) out.put(',');
        if (frame.object) {
          Key key = *key_at(frame.next++);
          write_key(L, out, key);
          out.put(':');
          push_key(L, key);
          lua_rawget(L, frame.slot);
        } else {
          lua_rawgeti(L, frame.slot, frame.next++);
        }
        return true;
      }
      if (frame.object) {
        out.put('}');
        keys.length = (size_t)frame.first * sizeof(Key);
      } else {
        out.put(']');
      }
      // a nested table is the value its parent pushed, on top of the stack
      if (frame.slot != 1) lua_pop(L, 1);
      depth--;
    }
    return false;
  }

  // write the value at `index`: a scalar now, a table by opening its frame. A scalar its parent pushed is popped
  void write(int index) {
    switch (lua_type(L, index)) {
      case LUA_TNIL:
        out.append("null", 4);
        break;
      case LUA_TBOOLEAN:
        if (lua_toboolean(L, index)) out.append("true", 4);
        else out.append("false", 5);
        break;
      case LUA_TNUMBER:
        write_number(L, out, index);
        break;
      case LUA_TSTRING: {
        size_t length;
        const char *text = lua_tolstring(L, index, &length);
        write_string(L, out, text, length);
        break;
      }
      case LUA_TTABLE:
        if (open(index)) return;
        break;
      default:
        fail(L, "type '%s' is not supported by JSON.", luaL_typename(L, index));
    }
    if (index != 1) lua_pop(L, 1);
  }
};

// json.encode(value): the JSON text of `value`
int json_encode(lua_State *L) {
  lua_settop(L, 1);
  CollectorPause pause(L);
  Buffer out(L, 256);
  Buffer keys(L, 16 * sizeof(Key));
  Frame frames[MAX_DEPTH];
  Encoder encoder = {L, out, keys, frames, 0};
  encoder.write(1);
  while (encoder.advance()) encoder.write(lua_gettop(L));
  lua_pushlstring(L, out.data, out.length);
  return 1;
}

// ---- decode -------------------------------------------------------------------------------------------------------

struct Decoder {
  lua_State *L;
  const char *start;
  const char *end;  // the NUL Lua keeps after the text's last byte
  Buffer &scratch;  // a string with escapes, unescaped

  // raise "invalid JSON: <what> at line L, column C", the position in bytes from 1, as dkjson wrote it
  [[noreturn]] void error(const char *at, const char *what) {
    lua_Integer line = 1;
    const char *line_start = start;
    for (const char *p = start; p < at; p++) {
      if (*p == '\n') {
        line++;
        line_start = p + 1;
      }
    }
    fail(L, "invalid JSON: %s at line %I, column %I", what, line, (lua_Integer)(at - line_start + 1));
  }

  static const char *skip_space(const char *p) {
    while (*p == ' ' || *p == '\n' || *p == '\r' || *p == '\t') p++;
    return p;
  }

  static int hex_digit(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
  }

  // the four hex digits at `p` as a number, or -1; stops at the first byte that is not one, so never passes the NUL
  static long hex4(const char *p) {
    long value = 0;
    for (int i = 0; i < 4; i++) {
      int digit = hex_digit(p[i]);
      if (digit < 0) return -1;
      value = value * 16 + digit;
    }
    return value;
  }

  // a byte of a string that is neither a quote nor a backslash nor ASCII text: the end, a control character, or the
  // start of a UTF-8 sequence, whose length is returned
  size_t check_byte(const char *open, const char *p) {
    unsigned char c = (unsigned char)*p;
    if (c < 0x20) {
      if (p == end) error(open, "unterminated string");
      error(p, "control character in string");
    }
    uint32_t code;
    size_t count = utf8_sequence((const unsigned char *)p, &code);
    if (!count) error(p, "invalid UTF-8");
    return count;
  }

  // the string whose opening quote is at `open`, pushed; returns the position after its closing quote
  const char *string(const char *open) {
    const char *p = open + 1, *run = p;
    // most strings have no escape: they are pushed straight from the text
    for (;;) {
      unsigned char c = (unsigned char)*p;
      if (c == '"') {
        lua_pushlstring(L, run, p - run);
        return p + 1;
      }
      if (c == '\\') break;
      if (c >= 0x20 && c < 0x80) p++;
      else p += check_byte(open, p);
    }
    scratch.length = 0;
    for (;;) {
      unsigned char c = (unsigned char)*p;
      if (c == '"') {
        scratch.append(run, p - run);
        lua_pushlstring(L, scratch.data, scratch.length);
        return p + 1;
      }
      if (c != '\\') {
        if (c >= 0x20 && c < 0x80) p++;
        else p += check_byte(open, p);
        continue;
      }
      scratch.append(run, p - run);
      char escape = p[1], single = 0;
      switch (escape) {
        case '"': single = '"'; break;
        case '\\': single = '\\'; break;
        case '/': single = '/'; break;
        case 'b': single = '\b'; break;
        case 'f': single = '\f'; break;
        case 'n': single = '\n'; break;
        case 'r': single = '\r'; break;
        case 't': single = '\t'; break;
        case 'u': break;
        default:
          if (p + 1 == end) error(open, "unterminated string");
          error(p, "invalid escape");
      }
      if (single) {
        scratch.put(single);
        p += 2;
      } else {
        long code = hex4(p + 2);
        if (code < 0) error(p, "invalid \\u escape");
        const char *next = p + 6;
        if (code >= 0xD800 && code <= 0xDBFF) {
          // a UTF-16 high surrogate, which a low one must follow
          long low = next[0] == '\\' && next[1] == 'u' ? hex4(next + 2) : -1;
          if (low < 0xDC00 || low > 0xDFFF) error(p, "unpaired surrogate in \\u escape");
          code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
          next += 6;
        } else if (code >= 0xDC00 && code <= 0xDFFF) {
          error(p, "unpaired surrogate in \\u escape");
        }
        char bytes[4];
        size_t count;
        if (code < 0x80) {
          bytes[0] = (char)code;
          count = 1;
        } else if (code < 0x800) {
          bytes[0] = (char)(0xC0 | (code >> 6));
          bytes[1] = (char)(0x80 | (code & 0x3F));
          count = 2;
        } else if (code < 0x10000) {
          bytes[0] = (char)(0xE0 | (code >> 12));
          bytes[1] = (char)(0x80 | ((code >> 6) & 0x3F));
          bytes[2] = (char)(0x80 | (code & 0x3F));
          count = 3;
        } else {
          bytes[0] = (char)(0xF0 | (code >> 18));
          bytes[1] = (char)(0x80 | ((code >> 12) & 0x3F));
          bytes[2] = (char)(0x80 | ((code >> 6) & 0x3F));
          bytes[3] = (char)(0x80 | (code & 0x3F));
          count = 4;
        }
        scratch.append(bytes, count);
        p = next;
      }
      run = p;
    }
  }

  // the number at `p`, pushed; returns the position after it
  const char *number(const char *p) {
    static const double POWERS[] = {1e0,  1e1,  1e2,  1e3,  1e4,  1e5,  1e6,  1e7,  1e8,  1e9,  1e10, 1e11,
                                    1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22};
    const char *start = p;
    bool negative = *p == '-';
    if (negative) p++;
    // the value is mantissa * 10^exponent while `exact`: up to 19 significant digits fit the mantissa
    unsigned long long mantissa = 0;
    int digits = 0;
    long exponent = 0;
    bool exact = true;
    if (*p == '0') {
      p++;
      if (*p >= '0' && *p <= '9') error(start, "invalid number (leading zero)");
    } else if (*p >= '1' && *p <= '9') {
      for (; *p >= '0' && *p <= '9'; p++) {
        if (digits < 19) {
          mantissa = mantissa * 10 + (unsigned long long)(*p - '0');
          digits++;
        } else {
          exact = false;
        }
      }
    } else {
      error(start, "invalid number");
    }
    bool integral = true;
    if (*p == '.') {
      integral = false;
      p++;
      if (!(*p >= '0' && *p <= '9')) error(start, "invalid number");
      for (; *p >= '0' && *p <= '9'; p++) {
        if (digits == 0 && *p == '0') {
          exponent--;  // a leading zero of the fraction
        } else if (digits < 19) {
          mantissa = mantissa * 10 + (unsigned long long)(*p - '0');
          digits++;
          exponent--;
        } else {
          exact = false;
        }
      }
    }
    if (*p == 'e' || *p == 'E') {
      integral = false;
      p++;
      bool down = *p == '-';
      if (*p == '+' || *p == '-') p++;
      if (!(*p >= '0' && *p <= '9')) error(start, "invalid number");
      long written = 0;
      for (; *p >= '0' && *p <= '9'; p++) {
        if (written < 100000) written = written * 10 + (*p - '0');
      }
      exponent += down ? -written : written;
    }
    if (integral && exact) {
      // an integer within 64 bits is a Lua integer
      unsigned long long limit = negative ? 9223372036854775808ULL : 9223372036854775807ULL;
      if (mantissa <= limit) {
        lua_pushinteger(L, negative ? (lua_Integer)(0ULL - mantissa) : (lua_Integer)mantissa);
        return p;
      }
    }
    double value;
    if (exact && mantissa <= (1ULL << 53) && exponent >= -22 && exponent <= 22) {
      // the mantissa and the power of ten are both exact doubles, so one operation rounds correctly (Clinger)
      value = exponent < 0 ? (double)mantissa / POWERS[-exponent] : (double)mantissa * POWERS[exponent];
      if (negative) value = -value;
    } else {
      // the text is a valid JSON number, which strtod reads in full, and what follows it cannot extend it
      value = strtod(start, NULL);
    }
    if (!isfinite(value)) error(start, "number out of range");
    lua_pushnumber(L, value);
    return p;
  }

  // the literal `word` at `p`, which must not run on into letters or digits; returns the position after it
  const char *literal(const char *p, const char *word, size_t length) {
    if (strncmp(p, word, length) != 0) error(p, "no valid JSON value");
    char next = p[length];
    if ((next >= 'a' && next <= 'z') || (next >= 'A' && next <= 'Z') || (next >= '0' && next <= '9')) {
      error(p, "no valid JSON value");
    }
    return p + length;
  }
};

// an array or object being read: where it opened (for "unterminated") and how many elements it has
struct Open {
  const char *at;
  lua_Integer count;
  bool object;
};

// json.decode(text): the value of the JSON text
int json_decode(lua_State *L) {
  lua_settop(L, 1);
  if (lua_type(L, 1) != LUA_TSTRING) fail(L, "json.decode expects a string, got %s", luaL_typename(L, 1));
  size_t length;
  const char *text = lua_tolstring(L, 1, &length);
  CollectorPause pause(L);
  Buffer scratch(L, 64);
  Decoder d = {L, text, text + length, scratch};
  Open opens[MAX_DEPTH];
  int depth = 0;
  const char *p = Decoder::skip_space(text);

value:
  // a value starts at p
  switch (*p) {
    case '{':
    case '[': {
      bool object = *p == '{';
      if (depth == MAX_DEPTH) d.error(p, "nested deeper than 1000 levels");
      luaL_checkstack(L, 4, "nesting too deep");
      lua_createtable(L, 0, 0);
      lua_pushvalue(L, object ? OBJECT_META : ARRAY_META);
      lua_setmetatable(L, -2);
      opens[depth++] = {p, 0, object};
      p = Decoder::skip_space(p + 1);
      if (*p == (object ? '}' : ']')) {
        p++;
        depth--;
        goto done;
      }
      if (object) goto key;
      goto value;
    }
    case '"':
      p = d.string(p);
      goto done;
    case '-':
    case '0':
    case '1':
    case '2':
    case '3':
    case '4':
    case '5':
    case '6':
    case '7':
    case '8':
    case '9':
      p = d.number(p);
      goto done;
    case 't':
      p = d.literal(p, "true", 4);
      lua_pushboolean(L, 1);
      goto done;
    case 'f':
      p = d.literal(p, "false", 5);
      lua_pushboolean(L, 0);
      goto done;
    case 'n':
      p = d.literal(p, "null", 4);
      lua_pushvalue(L, NULL_VALUE);
      goto done;
    default:
      if (p == d.end) {
        if (depth == 0) fail(L, "invalid JSON: no valid JSON value (reached the end)");
        d.error(opens[depth - 1].at, opens[depth - 1].object ? "unterminated object" : "unterminated array");
      }
      d.error(p, "no valid JSON value");
  }

key:
  // a key of the innermost object starts at p
  if (*p != '"') {
    if (p == d.end) d.error(opens[depth - 1].at, "unterminated object");
    d.error(p, "invalid key");
  }
  p = Decoder::skip_space(d.string(p));
  if (*p != ':') {
    if (p == d.end) d.error(opens[depth - 1].at, "unterminated object");
    d.error(p, "missing colon");
  }
  p = Decoder::skip_space(p + 1);
  goto value;

done:
  // a value is on top of the stack and p is past it
  if (depth > 0) {
    Open &open = opens[depth - 1];
    if (open.object) lua_rawset(L, -3);
    else lua_rawseti(L, -2, ++open.count);
    p = Decoder::skip_space(p);
    if (*p == ',') {
      p = Decoder::skip_space(p + 1);
      if (open.object) goto key;
      goto value;
    }
    if (*p == (open.object ? '}' : ']')) {
      p++;
      depth--;
      goto done;
    }
    if (p == d.end) d.error(open.at, open.object ? "unterminated object" : "unterminated array");
    d.error(p, open.object ? "expected ',' or '}'" : "expected ',' or ']'");
  }
  p = Decoder::skip_space(p);
  if (p != d.end) fail(L, "invalid JSON: unexpected data at byte %I", (lua_Integer)(p - text + 1));
  return 1;
}

}  // namespace

// Push the codec as a table: `null`, the `objectmeta` and `arraymeta` markers, and `encode` and `decode`, closures
// over those three.
int json_open(lua_State *L) {
  lua_createtable(L, 0, 5);
  // json.null: a table of its own, which decoding gives for null and encoding writes as null
  lua_newtable(L);
  lua_createtable(L, 0, 1);
  lua_pushliteral(L, "json.null");
  lua_setfield(L, -2, "__name");
  lua_setmetatable(L, -2);
  lua_createtable(L, 0, 1);
  lua_pushliteral(L, "object");
  lua_setfield(L, -2, "__jsontype");
  lua_createtable(L, 0, 1);
  lua_pushliteral(L, "array");
  lua_setfield(L, -2, "__jsontype");
  // stack: codec, null, objectmeta, arraymeta
  for (int i = 0; i < 2; i++) {
    lua_pushvalue(L, -3);
    lua_pushvalue(L, -3);
    lua_pushvalue(L, -3);
    lua_pushcclosure(L, i == 0 ? json_encode : json_decode, 3);
    lua_setfield(L, -5, i == 0 ? "encode" : "decode");
  }
  lua_setfield(L, -4, "arraymeta");
  lua_setfield(L, -3, "objectmeta");
  lua_setfield(L, -2, "null");
  return 1;
}
