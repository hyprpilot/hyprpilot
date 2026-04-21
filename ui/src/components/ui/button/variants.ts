import { cva, type VariantProps } from 'class-variance-authority'

export const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-theme-accent disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        accent: 'bg-theme-accent/15 text-theme-accent border border-theme-accent/40 hover:bg-theme-pending/20 hover:text-theme-pending hover:border-theme-pending/50',
        approve: 'bg-theme-idle/15 text-theme-idle border border-theme-idle/40 hover:bg-theme-pending/20 hover:text-theme-pending hover:border-theme-pending/50',
        reject: 'bg-theme-pending/15 text-theme-pending border border-theme-pending/40 hover:bg-theme-accent/20 hover:text-theme-accent hover:border-theme-accent/50',
        muted: 'bg-theme-fg-dim/10 text-theme-fg-dim border border-theme-border-soft hover:text-theme-fg'
      },
      size: {
        sm: 'h-7 px-2 text-xs tracking-wide',
        md: 'h-8 px-3 text-sm tracking-wide',
        lg: 'h-10 px-4 text-base tracking-wide'
      }
    },
    defaultVariants: {
      variant: 'accent',
      size: 'md'
    }
  }
)

export type ButtonVariants = VariantProps<typeof buttonVariants>
