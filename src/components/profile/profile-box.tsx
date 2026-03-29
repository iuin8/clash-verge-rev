import { alpha, Box, styled } from '@mui/material'

// FORK: isPrimary distinguishes the primary card in multi-merge mode with warning colour
// FORK: isDragging adds visual feedback during drag operations
export const ProfileBox = styled(Box)<{
  isPrimary?: boolean
  isDragging?: boolean
}>(({ theme, 'aria-selected': selected, isPrimary, isDragging }) => {
  const { mode, primary, warning, text } = theme.palette
  const accentColor = isPrimary && selected ? warning.main : primary.main
  const key = `${mode}-${!!selected}`

  const backgroundColor = mode === 'light' ? '#ffffff' : '#282A36'

  const color = {
    'light-true': text.secondary,
    'light-false': text.secondary,
    'dark-true': alpha(text.secondary, 0.65),
    'dark-false': alpha(text.secondary, 0.65),
  }[key]!

  const h2color = selected ? accentColor : text.primary

  const borderSelect = selected
    ? {
        borderLeft: `3px solid ${accentColor}`,
        width: `calc(100% + 3px)`,
        marginLeft: `-3px`,
      }
    : { width: '100%' }

  return {
    position: 'relative',
    display: 'block',
    cursor: 'pointer',
    textAlign: 'left',
    padding: '8px 16px',
    boxSizing: 'border-box',
    backgroundColor,
    ...borderSelect,
    borderRadius: '8px',
    color,
    '& h2': { color: h2color },
    ...(isDragging && {
      boxShadow: theme.shadows[8],
      transform: 'scale(1.02)',
      opacity: 0.9,
    }),
  }
})
