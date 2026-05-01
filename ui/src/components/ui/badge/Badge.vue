<script setup lang="ts">
import type { HTMLAttributes } from "vue"

import type { BadgeVariants } from "."
import { badgeVariants } from "."
import { cn } from '@components/lib/utils'

const props = defineProps<{
  variant?: BadgeVariants["variant"]
  class?: HTMLAttributes["class"]
}>()
</script>

<template>
  <div :class="cn(badgeVariants({ variant }), props.class)">
    <slot />
  </div>
</template>
