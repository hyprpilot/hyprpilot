<script setup lang="ts">
import { Primitive } from 'reka-ui'
import type { PrimitiveProps } from 'reka-ui'
import { computed } from 'vue'
import type { HTMLAttributes } from 'vue'

import { buttonVariants } from './variants'
import type { ButtonVariants } from './variants'
import { cn } from '@lib'

interface Props extends PrimitiveProps {
  variant?: ButtonVariants['variant']
  size?: ButtonVariants['size']
  class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<Props>(), {
  as: 'button'
})

const classes = computed(() => cn(buttonVariants({ variant: props.variant, size: props.size }), props.class))
</script>

<template>
  <Primitive :as="as" :as-child="asChild" :class="classes">
    <slot />
  </Primitive>
</template>
