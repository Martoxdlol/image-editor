import { describe, expect, it } from 'vitest'
import { colorToHex, paramBool, paramNumber, paramString } from './types'

describe('colorToHex', () => {
  it('converts linear-light components to sRGB hex', () => {
    expect(colorToHex({ rgba: [1, 1, 1, 1] })).toBe('#ffffff')
    expect(colorToHex({ rgba: [0, 0, 0, 1] })).toBe('#000000')
    // linear 0.2158 ≈ sRGB 128 (mid gray)
    expect(colorToHex({ rgba: [0.2158, 0.2158, 0.2158, 1] })).toBe('#808080')
  })

  it('clamps HDR values at display transform', () => {
    expect(colorToHex({ rgba: [2.5, 1.2, -0.3, 1] })).toBe('#ffff00')
  })
})

describe('param accessors', () => {
  it('reads typed param envelopes', () => {
    expect(paramNumber({ t: 'f64', v: 42 }, 0)).toBe(42)
    expect(paramNumber(undefined, 7)).toBe(7)
    expect(paramBool({ t: 'bool', v: true }, false)).toBe(true)
    expect(paramString({ t: 'str', v: 'hi' })).toBe('hi')
    expect(paramString({ t: 'expr', v: '$gridSize * 2' })).toBe('=$gridSize * 2')
    expect(paramString({ t: 'f64', v: 1 }, 'x')).toBe('x')
  })
})
