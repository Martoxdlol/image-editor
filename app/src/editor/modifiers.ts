// The modifier catalog (spec §2.2, §6.5/§6.6) — one source for the menu
// bar and the properties panel.

export interface ModifierDef {
  kind: string
  label: string
}

export const MODIFIER_GROUPS: { group: string; items: ModifierDef[] }[] = [
  {
    group: 'Geometry',
    items: [
      { kind: 'transform', label: 'Transform' },
      { kind: 'clip', label: 'Clip' },
    ],
  },
  {
    group: 'Filters',
    items: [
      { kind: 'filter.gaussian-blur', label: 'Gaussian Blur' },
      { kind: 'filter.pixelate', label: 'Pixelate' },
      { kind: 'filter.noise', label: 'Add Noise' },
    ],
  },
  {
    group: 'Adjustments',
    items: [
      { kind: 'adjust.brightness-contrast', label: 'Brightness / Contrast' },
      { kind: 'adjust.hsl', label: 'Hue / Saturation / Lightness' },
      { kind: 'adjust.levels', label: 'Levels' },
      { kind: 'adjust.invert', label: 'Invert' },
      { kind: 'adjust.grayscale', label: 'Grayscale' },
      { kind: 'adjust.posterize', label: 'Posterize' },
      { kind: 'adjust.threshold', label: 'Threshold' },
    ],
  },
]
