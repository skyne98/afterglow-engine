export interface FacePreset {
  name: string;
  weights: Readonly<Record<string, number>>;
}

export const EXPRESSION_PRESETS: readonly FacePreset[] = [
  { name: 'Neutral', weights: {} },
  {
    name: 'Happy',
    weights: {
      mouthSmileLeft: 0.85,
      mouthSmileRight: 0.85,
      cheekSquintLeft: 0.35,
      cheekSquintRight: 0.35,
      eyeSquintLeft: 0.15,
      eyeSquintRight: 0.15,
    },
  },
  {
    name: 'Big smile',
    weights: {
      mouthSmileLeft: 1,
      mouthSmileRight: 1,
      mouthStretchLeft: 0.2,
      mouthStretchRight: 0.2,
      cheekSquintLeft: 0.55,
      cheekSquintRight: 0.55,
      eyeSquintLeft: 0.2,
      eyeSquintRight: 0.2,
    },
  },
  {
    name: 'Sad',
    weights: {
      browInnerUp: 0.65,
      browDownLeft: 0.15,
      browDownRight: 0.15,
      mouthFrownLeft: 0.75,
      mouthFrownRight: 0.75,
      mouthShrugLower: 0.2,
    },
  },
  {
    name: 'Angry',
    weights: {
      browDownLeft: 0.85,
      browDownRight: 0.85,
      eyeSquintLeft: 0.4,
      eyeSquintRight: 0.4,
      noseSneerLeft: 0.35,
      noseSneerRight: 0.35,
      jawForward: 0.2,
      mouthPressLeft: 0.5,
      mouthPressRight: 0.5,
    },
  },
  {
    name: 'Surprised',
    weights: {
      browInnerUp: 0.9,
      browOuterUpLeft: 0.8,
      browOuterUpRight: 0.8,
      eyeWideLeft: 0.85,
      eyeWideRight: 0.85,
      jawOpen: 0.75,
    },
  },
  {
    name: 'Fear',
    weights: {
      browInnerUp: 0.85,
      browOuterUpLeft: 0.45,
      browOuterUpRight: 0.45,
      eyeWideLeft: 0.8,
      eyeWideRight: 0.8,
      mouthStretchLeft: 0.45,
      mouthStretchRight: 0.45,
      jawOpen: 0.35,
    },
  },
  {
    name: 'Disgust',
    weights: {
      noseSneerLeft: 0.8,
      noseSneerRight: 0.8,
      mouthFrownLeft: 0.45,
      mouthFrownRight: 0.45,
      mouthUpperUpLeft: 0.35,
      mouthUpperUpRight: 0.35,
      eyeSquintLeft: 0.25,
      eyeSquintRight: 0.25,
    },
  },
  {
    name: 'Smirk left',
    weights: {
      mouthSmileLeft: 0.75,
      mouthDimpleLeft: 0.5,
      eyeSquintLeft: 0.15,
    },
  },
  {
    name: 'Smirk right',
    weights: {
      mouthSmileRight: 0.75,
      mouthDimpleRight: 0.5,
      eyeSquintRight: 0.15,
    },
  },
  {
    name: 'Pucker',
    weights: { mouthPucker: 0.9, mouthFunnel: 0.3 },
  },
  {
    name: 'Cheek puff',
    weights: { cheekPuff: 1, mouthClose: 0.35 },
  },
  {
    name: 'Blink',
    weights: { eyeBlinkLeft: 1, eyeBlinkRight: 1 },
  },
  {
    name: 'Wink left',
    weights: { eyeBlinkLeft: 1, cheekSquintLeft: 0.25 },
  },
  {
    name: 'Wink right',
    weights: { eyeBlinkRight: 1, cheekSquintRight: 0.25 },
  },
  {
    name: 'Open mouth',
    weights: { jawOpen: 0.85 },
  },
  {
    name: 'Tongue out',
    weights: { jawOpen: 0.55, tongueOut: 1 },
  },
];

export const META_VISEMES: readonly FacePreset[] = [
  ['AA', 'viseme_aa'],
  ['CH', 'viseme_CH'],
  ['DD', 'viseme_DD'],
  ['E', 'viseme_E'],
  ['FF', 'viseme_FF'],
  ['I', 'viseme_I'],
  ['KK', 'viseme_kk'],
  ['NN', 'viseme_nn'],
  ['O', 'viseme_O'],
  ['PP', 'viseme_PP'],
  ['RR', 'viseme_RR'],
  ['SS', 'viseme_SS'],
  ['TH', 'viseme_TH'],
  ['U', 'viseme_U'],
].map(([name, target]) => ({ name: `Meta ${name}`, weights: { [target]: 1 } }));

export const MICROSOFT_VISEMES: readonly FacePreset[] = [
  ['AA / AH / AX', 'aa_ah_ax_01'],
  ['AA', 'aa_02'],
  ['AO', 'ao_03'],
  ['AW', 'aw_09'],
  ['AY', 'ay_11'],
  ['D / T / N', 'd_t_n_19'],
  ['ER', 'er_05'],
  ['EY / EH / UH', 'ey_eh_uh_04'],
  ['F / V', 'f_v_18'],
  ['H', 'h_12'],
  ['K / G / NG', 'k_g_ng_20'],
  ['L', 'l_14'],
  ['OW', 'ow_08'],
  ['OY', 'oy_10'],
  ['P / B / M', 'p_b_m_21'],
  ['R', 'r_13'],
  ['SH / CH / JH / ZH', 'sh_ch_jh_zh_16'],
  ['S / Z', 's_z_15'],
  ['TH / DH', 'th_dh_17'],
  ['W / UW', 'w_uw_07'],
  ['Y / IY / IH / IX', 'y_iy_ih_ix_06'],
].map(([name, target]) => ({ name: `Microsoft ${name}`, weights: { [target]: 1 } }));

export const SPEECH_PRESETS: readonly FacePreset[] = [
  { name: 'Speech neutral', weights: {} },
  ...META_VISEMES,
  ...MICROSOFT_VISEMES,
];
