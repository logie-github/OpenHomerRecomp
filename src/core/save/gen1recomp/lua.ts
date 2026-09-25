// Reader and writer for Gen1Recomp's save.lua format (src/core/SaveSerializer.lua).
//
// A save is `return <table>` made only of literals, %q strings and keyed tables.
// The writer mirrors upstream's: keys sorted (numbers before strings), two-space
// indentation, and a trailing comma after every entry, so an untouched save
// serializes back to the same text.

export type LuaKey = string | number
export type LuaValue = string | number | boolean | LuaTable
export type LuaTable = Map<LuaKey, LuaValue>

export function isLuaTable(value: LuaValue | undefined): value is LuaTable {
  return value instanceof Map
}

export function luaString(table: LuaTable | undefined, key: LuaKey): string | undefined {
  const value = table?.get(key)
  return typeof value === 'string' ? value : undefined
}

export function luaNumber(table: LuaTable | undefined, key: LuaKey): number | undefined {
  const value = table?.get(key)
  return typeof value === 'number' ? value : undefined
}

export function luaTable(table: LuaTable | undefined, key: LuaKey): LuaTable | undefined {
  const value = table?.get(key)
  return isLuaTable(value) ? value : undefined
}

/** The 1..n array part of a table, as upstream's ipairs would walk it. */
export function luaArray(table: LuaTable | undefined): LuaValue[] {
  const out: LuaValue[] = []
  if (!table) return out
  for (let i = 1; table.has(i); i++) {
    out.push(table.get(i) as LuaValue)
  }
  return out
}

export function luaArrayFrom(values: LuaValue[]): LuaTable {
  return new Map(values.map((value, i) => [i + 1, value]))
}

export function deepCopyLua<V extends LuaValue>(value: V): V {
  if (!isLuaTable(value)) return value
  return new Map([...value].map(([k, v]) => [k, deepCopyLua(v)])) as V
}

// ------------------------------------------------------------------ reader

const MAX_DEPTH = 128

const ESCAPES: Record<string, number> = {
  '"': 0x22,
  '\\': 0x5c,
  n: 0x0a,
  r: 0x0d,
  t: 0x09,
  a: 0x07,
  b: 0x08,
  f: 0x0c,
  v: 0x0b,
  '\n': 0x0a,
  '\r': 0x0a,
}

class LuaParser {
  private pos = 0
  private depth = 0
  private readonly encoder = new TextEncoder()
  private readonly decoder = new TextDecoder('utf-8')

  constructor(private readonly src: string) {}

  parse(): LuaTable {
    this.skip()
    if (!this.src.startsWith('return', this.pos)) this.fail('expected return')
    this.pos += 'return'.length
    const value = this.readValue()
    this.skip()
    if (this.pos < this.src.length) this.fail('trailing content')
    if (!isLuaTable(value)) this.fail('save root must be a table')
    return value
  }

  private fail(why: string): never {
    throw new Error(`save.lua parse error at ${this.pos}: ${why}`)
  }

  private skip() {
    while (this.pos < this.src.length && ' \t\r\n'.includes(this.src[this.pos])) this.pos++
  }

  private readValue(): LuaValue {
    this.skip()
    const c = this.src[this.pos]
    if (c === '"') return this.readString()
    if (c === '{') return this.readTable()
    if (c !== undefined && /[A-Za-z_]/.test(c)) {
      const word = this.readIdent()
      if (word === 'true') return true
      if (word === 'false') return false
      this.fail(`unexpected name '${word}'`)
    }
    if (c !== undefined && /[-\d.]/.test(c)) return this.readNumber()
    this.fail(c === undefined ? 'unexpected end of input' : 'unexpected character')
  }

  private readIdent(): string {
    const match = /^[A-Za-z_][A-Za-z0-9_]*/.exec(this.src.slice(this.pos, this.pos + 256))
    if (!match) this.fail('expected name')
    this.pos += match[0].length
    return match[0]
  }

  private readNumber(): number {
    const match = /^[^,\]}\s]+/.exec(this.src.slice(this.pos, this.pos + 64))
    const value = match ? Number(match[0]) : NaN
    if (!match || Number.isNaN(value)) this.fail('malformed number')
    this.pos += match[0].length
    return value
  }

  // Lua strings are bytes: literal characters contribute their UTF-8 encoding
  // and \ddd escapes contribute one byte each, then the whole is read as UTF-8.
  private readString(): string {
    const bytes: number[] = []
    let i = this.pos + 1
    let runStart = i
    const flush = (end: number) => {
      if (end > runStart) bytes.push(...this.encoder.encode(this.src.slice(runStart, end)))
    }
    for (;;) {
      const c = this.src[i]
      if (c === undefined) {
        this.pos = i
        this.fail('unterminated string')
      }
      if (c === '"') {
        flush(i)
        this.pos = i + 1
        return this.decoder.decode(new Uint8Array(bytes))
      }
      if (c === '\\') {
        flush(i)
        const next = this.src[i + 1]
        const digits = /^\d{1,3}/.exec(this.src.slice(i + 1, i + 4))
        if (digits) {
          const code = Number(digits[0])
          if (code > 255) {
            this.pos = i
            this.fail('escape out of range')
          }
          bytes.push(code)
          i += 1 + digits[0].length
        } else if (next !== undefined && next in ESCAPES) {
          bytes.push(ESCAPES[next])
          i += 2
        } else {
          this.pos = i
          this.fail('bad string escape')
        }
        runStart = i
      } else {
        i++
      }
    }
  }

  private readTable(): LuaTable {
    if (++this.depth > MAX_DEPTH) this.fail('table nesting too deep')
    this.pos++
    const out: LuaTable = new Map()
    this.skip()
    if (this.src[this.pos] === '}') {
      this.pos++
      this.depth--
      return out
    }
    for (;;) {
      this.skip()
      let key: LuaKey
      if (this.src[this.pos] === '[') {
        this.pos++
        const rawKey = this.readValue()
        if (typeof rawKey !== 'string' && typeof rawKey !== 'number') {
          this.fail('unsupported table key')
        }
        key = rawKey
        this.skip()
        if (this.src[this.pos] !== ']') this.fail('expected ]')
        this.pos++
      } else {
        key = this.readIdent()
      }
      this.skip()
      if (this.src[this.pos] !== '=') this.fail('expected =')
      this.pos++
      const value = this.readValue()
      if (out.has(key)) this.fail('duplicate table key')
      out.set(key, value)
      this.skip()
      const sep = this.src[this.pos]
      if (sep === ',') {
        this.pos++
        this.skip()
        if (this.src[this.pos] === '}') {
          this.pos++
          break
        }
      } else if (sep === '}') {
        this.pos++
        break
      } else {
        this.fail('expected , or }')
      }
    }
    this.depth--
    return out
  }
}

export function parseLuaSave(source: string): LuaTable {
  return new LuaParser(source).parse()
}

// ------------------------------------------------------------------ writer

const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) throw new Error(`cannot serialize ${n}`)
  return String(n)
}

// string.format('%q'): quotes, backslashes and control characters escaped.
function quote(s: string): string {
  let out = '"'
  for (const ch of s) {
    const code = ch.codePointAt(0) ?? 0
    if (ch === '"') out += '\\"'
    else if (ch === '\\') out += '\\\\'
    else if (ch === '\n') out += '\\n'
    else if (ch === '\r') out += '\\r'
    else if (code < 0x20 || code === 0x7f) out += '\\' + String(code).padStart(3, '0')
    else out += ch
  }
  return out + '"'
}

function compareKeys(a: LuaKey, b: LuaKey): number {
  const ta = typeof a
  const tb = typeof b
  if (ta !== tb) return ta < tb ? -1 : 1
  return a < b ? -1 : a > b ? 1 : 0
}

function serialize(value: LuaValue, indent: number): string {
  if (typeof value === 'number') return formatNumber(value)
  if (typeof value === 'boolean') return String(value)
  if (typeof value === 'string') return quote(value)
  if (value.size === 0) return '{}'
  const pad = '  '.repeat(indent)
  const parts = [...value.keys()].sort(compareKeys).map((k) => {
    const key =
      typeof k === 'string' && IDENT.test(k)
        ? k
        : `[${typeof k === 'number' ? formatNumber(k) : quote(k)}]`
    return `${pad}  ${key} = ${serialize(value.get(k) as LuaValue, indent + 1)}`
  })
  return `{\n${parts.join(',\n')},\n${pad}}`
}

export function serializeLuaSave(root: LuaTable): string {
  return `return ${serialize(root, 0)}\n`
}
