<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";

import {
  ALL_CONTRACTS,
  contractDownloadUrl,
  fetchContractBytes,
  fetchContractText,
} from "../api/docs";
import { ApiRequestError } from "../api/http";
import type { ContractName } from "../api/types";
import UiBadge from "../components/ui/UiBadge.vue";
import UiButton from "../components/ui/UiButton.vue";
import UiCard from "../components/ui/UiCard.vue";
import { useToast } from "../composables/useToast";

const { push } = useToast();

const selectedName = ref<ContractName>("grpc.proto");
const loading = ref(false);
const previewText = ref("");
const binaryByteLength = ref(0);

const selectedMeta = computed(
  () =>
    ALL_CONTRACTS.find((c) => c.name === selectedName.value) ??
    ALL_CONTRACTS[0]!,
);

async function loadContract(): Promise<void> {
  const current = selectedMeta.value;
  loading.value = true;
  previewText.value = "";
  binaryByteLength.value = 0;
  try {
    if (current.binary) {
      const bytes = await fetchContractBytes(current.name);
      binaryByteLength.value = bytes.byteLength;
    } else {
      previewText.value = await fetchContractText(current.name);
    }
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "契约读取失败", "error");
  } finally {
    loading.value = false;
  }
}

async function copyPreview(): Promise<void> {
  if (!previewText.value) {
    return;
  }
  try {
    await navigator.clipboard.writeText(previewText.value);
    push("已复制", "success");
  } catch {
    push("复制失败", "error");
  }
}

watch(selectedName, () => {
  void loadContract();
});

onMounted(() => {
  void loadContract();
});
</script>

<template>
  <div class="grid gap-6 lg:grid-cols-[300px_1fr]">
    <aside class="space-y-2">
      <div
        v-for="c in ALL_CONTRACTS"
        :key="c.name"
        class="flex items-center justify-between gap-2 rounded-lg border px-3 py-2.5 text-xs transition-colors"
        :class="
          selectedName === c.name
            ? 'border-slate-900 bg-slate-900 text-white'
            : 'border-slate-200 bg-white text-slate-800 hover:bg-slate-50'
        "
      >
        <button
          type="button"
          :data-testid="`contract-select-${c.name}`"
          class="min-w-0 flex-1 text-left"
          @click="selectedName = c.name"
        >
          <div class="truncate font-mono font-semibold">{{ c.name }}</div>
          <div
            class="truncate text-[11px]"
            :class="selectedName === c.name ? 'text-slate-300' : 'text-slate-500'"
          >
            {{ c.filename }}
          </div>
        </button>

        <a
          :href="contractDownloadUrl(c.name)"
          :download="c.filename"
          :data-testid="`contract-download-${c.name}`"
          class="shrink-0 rounded-md px-2.5 py-1 text-xs font-medium transition-colors"
          :class="
            selectedName === c.name
              ? 'bg-white text-slate-900 hover:bg-slate-100'
              : 'border border-slate-200 bg-slate-50 text-slate-700 hover:bg-slate-100'
          "
        >
          下载
        </a>
      </div>
    </aside>

    <UiCard :title="selectedMeta.name">
      <template #actions>
        <UiBadge tone="neutral">{{ selectedMeta.filename }}</UiBadge>
        <UiButton
          v-if="!selectedMeta.binary"
          size="sm"
          test-id="contract-copy-btn"
          @click="copyPreview"
        >
          复制
        </UiButton>
        <UiButton size="sm" :loading="loading" @click="loadContract">
          刷新
        </UiButton>
      </template>

      <div v-if="loading" class="py-10 text-center text-sm text-slate-500">
        加载中…
      </div>

      <div
        v-else-if="selectedMeta.binary"
        data-testid="contract-binary-summary"
        class="flex flex-col items-center justify-center gap-3 rounded-lg border border-slate-200 bg-slate-50 py-12 text-center"
      >
        <UiBadge tone="info">{{ binaryByteLength }} B</UiBadge>
        <p class="font-mono text-xs text-slate-600">{{ selectedMeta.filename }} (Protobuf FileDescriptorSet)</p>
        <a
          :href="contractDownloadUrl(selectedMeta.name)"
          :download="selectedMeta.filename"
          class="inline-flex items-center rounded-lg bg-slate-900 px-3.5 py-2 text-xs font-medium text-white hover:bg-slate-800"
        >
          下载
        </a>
      </div>

      <pre
        v-else
        data-testid="contract-preview"
        class="max-h-[70vh] overflow-auto rounded-lg bg-slate-900 p-4 font-mono text-xs leading-relaxed text-slate-100"
      >{{ previewText }}</pre>
    </UiCard>
  </div>
</template>
