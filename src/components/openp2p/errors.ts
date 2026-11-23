import type { TranslationKey } from '@/types/generated/i18n-keys'

type Translate = (key: TranslationKey) => string

const translateCataloguedCode = (
  key: TranslationKey,
  fallback: string,
  t: Translate,
) => {
  const translated = t(key)
  return translated === key ? fallback : translated
}

export const translateOpenP2pIssue = (
  code: string,
  fallback: string,
  t: Translate,
) =>
  translateCataloguedCode(
    `openp2p.errors.${code}` as TranslationKey,
    fallback,
    t,
  )

export const translateOpenP2pWarning = (code: string, t: Translate) =>
  translateCataloguedCode(
    `openp2p.import.warnings.${code}` as TranslationKey,
    code,
    t,
  )

/** Localizes backend error codes when the payload carries a bare code. */
export const translateOpenP2pErrorText = (value: string, t: Translate) =>
  translateCataloguedCode(`openp2p.errors.${value}` as TranslationKey, value, t)

interface FailureLike {
  code?: unknown
  detail?: unknown
}

export const openP2pFailureMessage = (
  error: unknown,
  fallback: string,
  t: Translate,
) => {
  const failure =
    typeof error === 'object' && error !== null
      ? (error as FailureLike)
      : undefined
  const code = typeof failure?.code === 'string' ? failure.code : undefined
  const detail =
    typeof failure?.detail === 'string'
      ? failure.detail
      : error instanceof Error
        ? error.message
        : typeof error === 'string'
          ? error
          : ''
  const localized = code
    ? translateCataloguedCode(`openp2p.errors.${code}` as TranslationKey, '', t)
    : ''
  return [localized, detail].filter(Boolean).join(' ') || fallback
}
