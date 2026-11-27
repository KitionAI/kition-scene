import type { Catalog } from '@kition/sdk/editor'

/** The number fields', the timeline's playback and auto-keying, and the material users' strings (phase 8). */
export const FIELD_EN: Catalog = {
  fieldCopy: 'Copy',
  fieldPaste: 'Paste',
  fieldReset: 'Reset to Default',
  fieldInsertKey: 'Insert Keyframe',
  fieldInvalid: '“{text}” is not a value',
  autoKey: 'Auto Keying',
  keyingMenu: 'Insert Keyframe',
  materialUsers: '{count} objects use this material: make a copy of it for this object only',
  materialAddColor: 'New material (click to make one)',
}

export const FIELD_ZH: Catalog = {
  fieldCopy: '复制',
  fieldPaste: '粘贴',
  fieldReset: '恢复默认值',
  fieldInsertKey: '插入关键帧',
  fieldInvalid: '“{text}”不是有效的数值',
  autoKey: '自动插帧',
  keyingMenu: '插入关键帧',
  materialUsers: '{count} 个物体共用这个材质：为这个物体复制一份独立的',
  materialAddColor: '新建材质（点一下新建）',
}
