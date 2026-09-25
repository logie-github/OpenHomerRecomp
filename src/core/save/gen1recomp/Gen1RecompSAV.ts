// A Gen1Recomp (Red/Blue/Yellow) save.lua, opened either from disk or through
// Gen1Recomp Save Sync. Its PC boxes are presented as PK1s so that every other
// part of OpenHome (tracking, conversion, storage) treats it like any other
// Generation I save.

import { PK1 } from '@openhome-core/pkm'
import { NationalDex } from '@openhome-core/resources/consts/NationalDex'
import { Errorable, Option, range, unique } from '@openhome-core/util/functional'
import { BinaryGender, ConvertStrategy, ExtraFormIndex, ItemGen1, OriginGame } from '@pkm-rs/pkg'
import { OHPKM } from '../../pkm/OHPKM'
import { Box, BoxAndSlot, OfficialSAV, SlotMetadata } from '../interfaces'
import { LookupType } from '../util'
import { PathData } from '../util/path'
import { luaMonToPk1, pk1ToLuaMon } from './gen1RecompMon'
import {
  isLuaTable,
  luaArray,
  luaArrayFrom,
  luaNumber,
  luaString,
  LuaTable,
  luaTable,
  LuaValue,
  parseLuaSave,
  serializeLuaSave,
} from './lua'

const BOX_CAPACITY = 20
const GEN1_VERSIONS: Record<string, OriginGame> = {
  red: OriginGame.Red,
  blue: OriginGame.BlueGreen,
  yellow: OriginGame.Yellow,
}
const GEN2_VERSIONS = ['gold', 'silver', 'crystal']

const decoder = new TextDecoder('utf-8')
const encoder = new TextEncoder()

/** Where a Generation I save keeps its PC: twelve `boxes`, or one legacy `box`. */
function boxesOf(root: LuaTable): { key: 'boxes' | 'box'; lists: LuaTable[] } | undefined {
  const boxes = luaTable(root, 'boxes')
  if (boxes) {
    return { key: 'boxes', lists: luaArray(boxes).map((b) => (isLuaTable(b) ? b : new Map())) }
  }
  const legacy = luaTable(root, 'box')
  return legacy ? { key: 'box', lists: [legacy] } : undefined
}

/** The game this save belongs to, or undefined if it is not a Generation I save.lua. */
function gen1Origin(root: LuaTable): OriginGame | undefined {
  const version = luaString(root, 'version')?.toLowerCase()
  if (version === undefined) {
    // Saves from before Blue support carry no version; upstream core
    // migration 2 treats every untagged one as Red.
    return luaNumber(root, 'generation') === 2 ? undefined : OriginGame.Red
  }
  if (GEN2_VERSIONS.includes(version)) return undefined
  return GEN1_VERSIONS[version]
}

function looksLikeLuaSave(bytes: Uint8Array): boolean {
  const head = decoder.decode(bytes.subarray(0, 64)).trimStart()
  return head.startsWith('return {') || head.startsWith('return{')
}

export class Gen1RecompSAV extends OfficialSAV<PK1> {
  static pkmType = PK1
  static lookupType: LookupType = 'gen12'

  static saveTypeAbbreviation = 'Gen1Recomp'
  static saveTypeName = 'Gen1Recomp (Red/Blue/Yellow)'
  static saveTypeID = 'Gen1RecompSAV'

  origin: OriginGame
  isPlugin: false = false

  boxRows = 4
  boxColumns = 5

  filePath: PathData
  fileCreated?: Date

  money: number
  name: string
  tid: number
  sid?: number | undefined
  displayID: string
  language = undefined

  currentPCBox: number
  boxes: Array<Box<PK1>>

  bytes: Uint8Array

  invalid: boolean = false
  tooEarlyToOpen: boolean = false

  updatedBoxSlots: BoxAndSlot[] = []

  root: LuaTable
  private boxesKey: 'boxes' | 'box'
  /** The save's own record for each slot, so untouched Pokémon are written back verbatim. */
  private rawSlots: Option<LuaTable>[][]
  /** Records OpenHome cannot represent as PK1 (a mod's species, say); these slots are locked. */
  private opaqueSlots: Set<string> = new Set()

  constructor(path: PathData, bytes: Uint8Array) {
    super()
    this.filePath = path
    this.bytes = bytes
    this.root = parseLuaSave(decoder.decode(bytes))

    const origin = gen1Origin(this.root)
    if (origin === undefined) throw new Error('Not a Generation I Gen1Recomp save')
    this.origin = origin

    const player = luaTable(this.root, 'player')
    this.name = luaString(player, 'name') ?? ''
    this.tid = luaNumber(player, 'id') ?? 0
    this.displayID = this.tid.toString().padStart(5, '0')
    this.money = luaNumber(this.root, 'money') ?? luaNumber(player, 'money') ?? 0

    const pc = boxesOf(this.root)
    if (!pc) throw new Error('This Gen1Recomp save has no PC boxes')
    this.boxesKey = pc.key
    this.currentPCBox = Math.max(0, (luaNumber(this.root, 'currentBox') ?? 1) - 1)
    if (this.currentPCBox >= pc.lists.length) this.currentPCBox = 0

    const context = { origin: this.origin, playerName: this.name }
    this.rawSlots = []
    this.boxes = pc.lists.map((list, boxNum) => {
      const records = luaArray(list)
      if (records.length > BOX_CAPACITY) {
        throw new Error(`Box ${boxNum + 1} holds more than ${BOX_CAPACITY} Pokémon`)
      }
      const box = new Box<PK1>(`Box ${boxNum + 1}`, BOX_CAPACITY)
      this.rawSlots[boxNum] = new Array(BOX_CAPACITY)
      records.forEach((record, slot) => {
        if (!isLuaTable(record)) return
        this.rawSlots[boxNum][slot] = record
        const mon = luaMonToPk1(record, context)
        if (mon) box.boxSlots[slot] = mon
        else this.opaqueSlots.add(`${boxNum}:${slot}`)
      })
      return box
    })
  }

  getSlotMetadata = (boxNum: number, boxSlot: number): SlotMetadata =>
    this.opaqueSlots.has(`${boxNum}:${boxSlot}`)
      ? { isDisabled: true, disabledReason: 'OpenHome cannot read this Pokémon' }
      : { isDisabled: false }

  prepareForSaving() {
    const changedBoxes = unique(this.updatedBoxSlots.map((coords) => coords.box))
    if (changedBoxes.length === 0) return

    const pc = boxesOf(this.root)
    if (!pc) return
    const lists = [...pc.lists]
    const context = { origin: this.origin, playerName: this.name }

    for (const boxNum of changedBoxes) {
      const records: LuaValue[] = []
      const written: Option<LuaTable>[] = new Array(BOX_CAPACITY)
      range(BOX_CAPACITY).forEach((slot) => {
        const raw = this.rawSlots[boxNum]?.[slot]
        if (this.opaqueSlots.has(`${boxNum}:${slot}`)) {
          if (raw) records.push(raw)
          written[slot] = raw
          return
        }
        const mon = this.boxes[boxNum]?.boxSlots[slot]
        if (!mon) return
        // A Pokémon that is exactly what the save already had keeps its own
        // record untouched, including any field only a mod knows about.
        // One that changed (healed, say) is rewritten over its own record;
        // anything else in this slot is a different Pokémon and starts fresh.
        const previous = raw ? luaMonToPk1(raw, context) : undefined
        const record =
          raw && previous && samePk1(previous, mon)
            ? raw
            : pk1ToLuaMon(mon, previous && sameIdentity(previous, mon) ? raw : undefined)
        records.push(record)
        written[slot] = record
      })
      // Gen1Recomp boxes are dense lists (table.insert/table.remove), so the
      // box is compacted just as G1SAV compacts a cartridge box. The slots
      // shown in OpenHome keep their positions, so the records they now
      // correspond to are remembered by those positions.
      lists[boxNum] = luaArrayFrom(records)
      this.rawSlots[boxNum] = written
    }

    if (this.boxesKey === 'boxes') {
      this.root.set('boxes', luaArrayFrom(lists))
    } else {
      this.root.set('box', lists[0])
    }
    this.bytes = encoder.encode(serializeLuaSave(this.root))
  }

  convertOhpkm(ohpkm: OHPKM, strategy: ConvertStrategy): Errorable<PK1> {
    return PK1.fromOhpkm(ohpkm, strategy)
  }

  supportsMon(nationalDex: number, formeNumber: number, extraFormIndex?: ExtraFormIndex): boolean {
    if (extraFormIndex !== undefined) return false
    return nationalDex <= NationalDex.Mew && formeNumber === 0
  }

  supportsItem(itemIndex: number) {
    return ItemGen1.fromModern(itemIndex) !== undefined
  }

  get trainerGender() {
    return BinaryGender.Male
  }

  getDisplayData() {
    return {
      'Trainer Name': this.name,
      'Trainer ID': this.displayID,
      Source: this.filePath.raw.startsWith(GEN1RECOMP_SYNC_SCHEME) ? 'Save Sync' : 'File',
    }
  }

  getMonAt(boxNum: number, boxSlot: number) {
    return this.boxes[boxNum]?.boxSlots[boxSlot]
  }

  setMonAt(boxNum: number, boxSlot: number, mon: Option<PK1>): void {
    const box = this.boxes[boxNum]
    if (!box) return
    box.boxSlots[boxSlot] = mon
  }

  static fileIsSave(bytes: Uint8Array): boolean {
    if (!looksLikeLuaSave(bytes)) return false
    try {
      const root = parseLuaSave(decoder.decode(bytes))
      return gen1Origin(root) !== undefined && boxesOf(root) !== undefined
    } catch {
      return false
    }
  }

  static includesOrigin(origin: OriginGame) {
    return origin >= OriginGame.Red && origin <= OriginGame.Yellow
  }
}

function sameIdentity(a: PK1, b: PK1): boolean {
  return (
    a.nationalDex === b.nationalDex &&
    a.trainerID === b.trainerID &&
    a.trainerName === b.trainerName &&
    a.dvs.atk === b.dvs.atk &&
    a.dvs.def === b.dvs.def &&
    a.dvs.spe === b.dvs.spe &&
    a.dvs.spc === b.dvs.spc
  )
}

function samePk1(a: PK1, b: PK1): boolean {
  const bytesA = new Uint8Array(a.toBytes())
  const bytesB = new Uint8Array(b.toBytes())
  return (
    a.nickname === b.nickname &&
    a.trainerName === b.trainerName &&
    a.currentHP === b.currentHP &&
    bytesA.length === bytesB.length &&
    bytesA.every((byte, i) => byte === bytesB[i])
  )
}

// ------------------------------------------------------------------ Save Sync paths
//
// A save opened through Save Sync has no file. It is addressed by a path in
// this scheme, which the backend routes to the sync commands instead of the
// filesystem: gen1recomp-sync://<version>/<playthroughId>

export const GEN1RECOMP_SYNC_SCHEME = 'gen1recomp-sync://'

export function gen1RecompSyncPath(
  version: string,
  playthroughId: string,
  label: string
): PathData {
  const dir = `${GEN1RECOMP_SYNC_SCHEME}${version}`
  return {
    raw: `${dir}/${playthroughId}`,
    dir,
    name: label,
    ext: '',
    separator: '/',
  }
}

export function parseGen1RecompSyncPath(
  raw: string
): { version: string; playthroughId: string } | undefined {
  if (!raw.startsWith(GEN1RECOMP_SYNC_SCHEME)) return undefined
  const rest = raw.slice(GEN1RECOMP_SYNC_SCHEME.length)
  const slash = rest.indexOf('/')
  if (slash <= 0 || slash === rest.length - 1) return undefined
  return { version: rest.slice(0, slash), playthroughId: rest.slice(slash + 1) }
}
