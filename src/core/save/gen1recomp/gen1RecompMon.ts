// Translates between one Pokémon as Gen1Recomp stores it in save.lua and
// OpenHome's existing Generation I representation (PK1).
//
// Field meanings follow Gen1Recomp's own .sav converter
// (src/save_convert/GenSave.lua decodeMon/encodeMon), which maps the same Lua
// table to and from the cartridge's party_struct bytes that PK1 reads.

import { PK1 } from '@openhome-core/pkm'
import { toGen1PokemonIndex } from '@openhome-core/pkm/conversion/Gen1PokemonIndex'
import { writeDVsToBytes } from '@openhome-core/util/types'
import { Language, metadataReaderFor, MetadataSource, OriginGame, PkmTypes } from '@pkm-rs/pkg'
import { GEN1_MOVES, GEN1_SPECIES } from './gen1Constants'
import {
  deepCopyLua,
  luaArray,
  luaArrayFrom,
  LuaTable,
  luaNumber,
  luaString,
  luaTable,
  LuaValue,
} from './lua'

const PARTY_STRUCT_SIZE = 0x2c

const DEX_BY_SPECIES = new Map(GEN1_SPECIES.map(([id], i) => [id, i + 1]))
const MOVE_INDEX = new Map(GEN1_MOVES.map((id, i) => [id, i + 1]))

// STATUS_* bits (pokered constants/battle_constants.asm), as GenSave.lua reads them
const STATUS_BITS: [string, number][] = [
  ['PSN', 3],
  ['BRN', 4],
  ['FRZ', 5],
  ['PAR', 6],
]

function encodeStatus(status: string | undefined): number {
  if (status === 'SLP') return 7
  const bit = STATUS_BITS.find(([name]) => name === status)?.[1]
  return bit === undefined ? 0 : 1 << bit
}

function decodeStatus(byte: number): string | undefined {
  if ((byte & 7) > 0) return 'SLP'
  return STATUS_BITS.find(([, bit]) => (byte & (1 << bit)) !== 0)?.[0]
}

/** The name the cartridge stores for a Pokémon that was never nicknamed. */
export function gen1SpeciesName(nationalDex: number): string | undefined {
  return GEN1_SPECIES[nationalDex - 1]?.[1]
}

function speciesConstant(nationalDex: number): string | undefined {
  return GEN1_SPECIES[nationalDex - 1]?.[0]
}

function defaultTypeBytes(nationalDex: number): [number, number] {
  const metadata = metadataReaderFor(MetadataSource.Yellow, nationalDex, 0)
  const type1 = metadata?.type1()
  const type2 = metadata?.type2()
  const t1 = type1 !== undefined ? PkmTypes.toGameboyIndex(type1) : 0
  const t2 = type2 !== undefined ? PkmTypes.toGameboyIndex(type2) : t1
  return [t1, t2]
}

function clamp(value: number | undefined, max: number): number {
  return Math.max(0, Math.min(max, Math.floor(value ?? 0)))
}

export interface MonContext {
  origin: OriginGame
  /** Written as the OT of a Pokémon whose record has none (GenSave.lua does the same). */
  playerName: string
}

/**
 * Reads a save.lua Pokémon as a PK1, or undefined if it is not one OpenHome can
 * represent (a species or move a mod added, an egg).
 */
export function luaMonToPk1(mon: LuaTable, context: MonContext): PK1 | undefined {
  if (mon.get('isEgg') === true) return undefined
  const species = luaString(mon, 'species')
  const nationalDex = species ? DEX_BY_SPECIES.get(species) : undefined
  if (!nationalDex) return undefined

  const moves = luaArray(luaTable(mon, 'moves')).map((move) =>
    move instanceof Map ? move : new Map<string, LuaValue>()
  )
  if (moves.length > 4) return undefined
  const moveIndices = moves.map((move) => MOVE_INDEX.get(luaString(move, 'id') ?? ''))
  if (moveIndices.some((index) => index === undefined)) return undefined

  const buffer = new ArrayBuffer(PARTY_STRUCT_SIZE)
  const view = new DataView(buffer)
  view.setUint8(0x0, toGen1PokemonIndex(nationalDex))
  // 0x1-0x2 (current HP) stays zero here: PK1's constructor treats a 0xFF in
  // byte 2 as a .pk1 file header. The real value is assigned below.
  view.setUint8(0x3, clamp(luaNumber(mon, 'level'), 100))
  view.setUint8(0x4, encodeStatus(luaString(mon, 'status')))
  const typeBytes = luaArray(luaTable(mon, 'typeBytes'))
  const [t1, t2] =
    typeBytes.length === 2 && typeof typeBytes[0] === 'number' && typeof typeBytes[1] === 'number'
      ? [typeBytes[0], typeBytes[1]]
      : defaultTypeBytes(nationalDex)
  view.setUint8(0x5, clamp(t1, 0xff))
  view.setUint8(0x6, clamp(t2, 0xff))
  view.setUint8(0x7, clamp(luaNumber(mon, 'catchRate') ?? GEN1_SPECIES[nationalDex - 1][2], 0xff))
  moves.forEach((move, i) => {
    view.setUint8(0x8 + i, moveIndices[i] ?? 0)
    const pp = clamp(luaNumber(move, 'pp'), 0x3f)
    const ppUps = clamp(luaNumber(move, 'ppUps'), 3)
    view.setUint8(0x1d + i, (ppUps << 6) | pp)
  })
  view.setUint16(0xc, clamp(luaNumber(mon, 'otId'), 0xffff), false)
  const exp = clamp(luaNumber(mon, 'exp'), 0xffffff)
  view.setUint8(0xe, (exp >> 16) & 0xff)
  view.setUint16(0xf, exp & 0xffff, false)
  const statExp = luaTable(mon, 'statExp')
  ;['hp', 'attack', 'defense', 'speed', 'special'].forEach((stat, i) =>
    view.setUint16(0x11 + i * 2, clamp(luaNumber(statExp, stat), 0xffff), false)
  )
  const dvs = luaTable(mon, 'dvs')
  writeDVsToBytes(
    {
      hp: 0,
      atk: clamp(luaNumber(dvs, 'attack'), 15),
      def: clamp(luaNumber(dvs, 'defense'), 15),
      spe: clamp(luaNumber(dvs, 'speed'), 15),
      spc: clamp(luaNumber(dvs, 'special'), 15),
    },
    view,
    0x1b
  )

  const pk1 = PK1.fromBytes(buffer)
  pk1.currentHP = Math.min(clamp(luaNumber(mon, 'hp'), 0xffff), pk1.getStats().hp)
  // Gen1Recomp spells "not nicknamed" as nickname == nil; the cartridge (and
  // PK1) spells it as the species' own name.
  pk1.nickname = luaString(mon, 'nickname') ?? gen1SpeciesName(nationalDex) ?? ''
  pk1.trainerName = luaString(mon, 'ot') ?? context.playerName
  pk1.gameOfOrigin = context.origin
  pk1.language = Language.English
  return pk1
}

/**
 * Writes a PK1 as a save.lua Pokémon. Starts from [base] (the record this
 * Pokémon had in the save, when it had one) so fields this adapter does not
 * know about, such as a mod's, survive.
 */
export function pk1ToLuaMon(pk1: PK1, base?: LuaTable): LuaTable {
  const species = speciesConstant(pk1.nationalDex)
  if (!species) throw new Error(`Gen1Recomp cannot hold national dex #${pk1.nationalDex}`)
  const mon: LuaTable = base ? deepCopyLua(base) : new Map()
  const stats = pk1.getStats()
  const level = pk1.getLevel()

  mon.set('species', species)
  mon.set('level', level)
  mon.set('exp', pk1.exp)
  mon.set('hp', Math.min(pk1.currentHP, stats.hp))
  const status = decodeStatus(pk1.statusCondition)
  if (status) mon.set('status', status)
  else mon.delete('status')
  mon.set('catchRate', pk1.heldItemIndexGen1?.index ?? GEN1_SPECIES[pk1.nationalDex - 1][2])
  mon.set('typeBytes', luaArrayFrom([pk1.type1, pk1.type2]))
  mon.set(
    'moves',
    luaArrayFrom(
      pk1.moves.flatMap((moveIndex, i) => {
        const id = GEN1_MOVES[moveIndex - 1]
        if (!id) return []
        const move: LuaTable = new Map<string, LuaValue>([
          ['id', id],
          ['pp', pk1.movePP[i]],
        ])
        if (pk1.movePPUps[i]) move.set('ppUps', pk1.movePPUps[i])
        return [move]
      })
    )
  )
  mon.set('otId', pk1.trainerID)
  mon.set('ot', pk1.trainerName)
  if (pk1.nickname && pk1.nickname !== gen1SpeciesName(pk1.nationalDex)) {
    mon.set('nickname', pk1.nickname)
  } else {
    mon.delete('nickname')
  }
  mon.set(
    'dvs',
    new Map<string, LuaValue>([
      ['hp', pk1.dvs.hp],
      ['attack', pk1.dvs.atk],
      ['defense', pk1.dvs.def],
      ['speed', pk1.dvs.spe],
      ['special', pk1.dvs.spc],
    ])
  )
  mon.set(
    'statExp',
    new Map<string, LuaValue>([
      ['hp', pk1.evsG12.hp],
      ['attack', pk1.evsG12.atk],
      ['defense', pk1.evsG12.def],
      ['speed', pk1.evsG12.spe],
      ['special', pk1.evsG12.spc],
    ])
  )
  mon.set(
    'stats',
    new Map<string, LuaValue>([
      ['hp', stats.hp],
      ['attack', stats.atk],
      ['defense', stats.def],
      ['speed', stats.spe],
      ['special', stats.spc],
    ])
  )
  return mon
}
