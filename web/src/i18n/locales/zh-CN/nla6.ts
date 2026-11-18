import type { Catalog } from '@kition/kit-editor'

export const NLA6_ZH: Catalog = {
  nlaAddTransition: '过渡到下一个',
  nlaGroupNext: '与下一个组合',
  nlaDissolve: '解散组合',
  nlaMetaEnd: '结束',
  nlaChildren: '组合中有 {count} 个片段',
  nlaTransitionHint: '时间范围跟随相邻片段。',
  fmod_limits: '限制',
  fmod_minX: '最小帧',
  fmod_maxX: '最大帧',
  fmod_minY: '最小值',
  fmod_maxY: '最大值',
}

export const NLA6_PHRASES_ZH: Record<string, string> = {
  'Add Transition Strip': '添加过渡片段',
  'Make Meta Strip': '创建组合片段',
  'Dissolve Meta Strip': '解散组合片段',
}
