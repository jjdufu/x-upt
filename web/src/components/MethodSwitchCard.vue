<script setup lang="ts">
import { computed } from "vue";

import type { MethodStatus } from "../api/types";
import UiBadge from "./ui/UiBadge.vue";
import UiCard from "./ui/UiCard.vue";
import UiSwitch from "./ui/UiSwitch.vue";

const props = withDefaults(
  defineProps<{
    item: MethodStatus;
    busy?: boolean;
  }>(),
  {
    busy: false,
  },
);

const emit = defineEmits<{
  toggle: [method: MethodStatus, next: boolean];
  gotoImport: [method: MethodStatus];
}>();

const switchDisabled = computed(
  () => props.busy || (!props.item.enabled && !props.item.can_enable),
);

const statusText = computed(() => {
  if (props.item.enabled) {
    return "运行中";
  }
  if (!props.item.can_enable) {
    return "未就绪";
  }
  return "已停用";
});

const statusTone = computed(() => {
  if (props.item.enabled) {
    return "success" as const;
  }
  if (!props.item.can_enable) {
    return "warning" as const;
  }
  return "neutral" as const;
});

function handleSwitch(next: boolean): void {
  if (switchDisabled.value) {
    return;
  }
  emit("toggle", props.item, next);
}
</script>

<template>
  <UiCard :test-id="`method-card-${item.method}`" padding="sm">
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <h3 class="text-sm font-semibold text-slate-900">
            {{ item.label }}
          </h3>
          <UiBadge :tone="statusTone" :test-id="`method-badge-${item.method}`">
            {{ statusText }}
          </UiBadge>
        </div>
        <p class="mt-1 font-mono text-xs text-slate-500">
          {{ item.method }}
        </p>
      </div>

      <UiSwitch
        :model-value="item.enabled"
        :disabled="switchDisabled"
        :test-id="`method-switch-${item.method}`"
        :aria-label="item.label"
        @update:model-value="handleSwitch"
      />
    </div>

    <div
      v-if="!item.enabled && item.missing_preconditions.length > 0"
      :data-testid="`method-missing-${item.method}`"
      class="mt-3 rounded-lg border border-amber-200 bg-amber-50/70 px-3 py-2 text-xs text-amber-900"
    >
      <ul class="space-y-1">
        <li v-for="(reason, idx) in item.missing_preconditions" :key="idx">
          {{ reason }}
        </li>
      </ul>
      <div class="mt-2 flex items-center gap-3">
        <button
          type="button"
          :data-testid="`method-goto-import-${item.method}`"
          class="font-medium text-amber-900 underline underline-offset-2 hover:text-slate-900"
          @click="emit('gotoImport', item)"
        >
          去导入
        </button>
      </div>
    </div>
  </UiCard>
</template>
