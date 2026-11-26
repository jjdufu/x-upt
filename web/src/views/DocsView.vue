<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { fetchFrontDocs } from "../api/docs";
import { ApiRequestError } from "../api/http";
import type { FrontDoc } from "../api/types";
import UiBadge from "../components/ui/UiBadge.vue";
import UiButton from "../components/ui/UiButton.vue";
import UiCard from "../components/ui/UiCard.vue";
import UiInput from "../components/ui/UiInput.vue";
import UiSelect from "../components/ui/UiSelect.vue";
import { useToast } from "../composables/useToast";

const { push } = useToast();

const categoryOptions = [
  { value: "all", label: "全部协议面" },
  { value: "core", label: "核心业务面" },
  { value: "envelope", label: "信封协议面" },
] as const;

const loading = ref(false);
const docs = ref<FrontDoc[]>([]);
const selectedName = ref("");
const categoryFilter = ref("all");
const searchKeyword = ref("");

const filteredDocs = computed(() => {
  const kw = searchKeyword.value.trim().toLowerCase();
  return docs.value.filter((d) => {
    if (categoryFilter.value !== "all" && d.category !== categoryFilter.value) {
      return false;
    }
    if (!kw) {
      return true;
    }
    return (
      d.name.toLowerCase().includes(kw) ||
      d.protocol.toLowerCase().includes(kw) ||
      d.summary.toLowerCase().includes(kw)
    );
  });
});

const activeDoc = computed<FrontDoc | undefined>(() => {
  const found = filteredDocs.value.find((d) => d.name === selectedName.value);
  return found ?? filteredDocs.value[0];
});

async function loadDocs(): Promise<void> {
  loading.value = true;
  try {
    const res = await fetchFrontDocs();
    docs.value = res.items;
    if (!selectedName.value && res.items[0]) {
      selectedName.value = res.items[0].name;
    }
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    loading.value = false;
  }
}

async function copyCode(content: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(content);
    push("已复制", "success");
  } catch {
    push("复制失败", "error");
  }
}

onMounted(() => {
  void loadDocs();
});
</script>

<template>
  <div class="grid gap-6 lg:grid-cols-[280px_1fr]">
    <aside class="space-y-3">
      <UiCard padding="sm">
        <div class="space-y-2.5">
          <UiSelect
            v-model="categoryFilter"
            :options="categoryOptions"
            label="分类"
            test-id="docs-category-select"
          />
          <UiInput
            v-model="searchKeyword"
            type="search"
            label="筛选"
            placeholder="rest / grpc / envelope"
            test-id="docs-search-input"
          />
          <UiButton
            size="sm"
            :loading="loading"
            test-id="docs-refresh-btn"
            class="w-full"
            @click="loadDocs"
          >
            刷新
          </UiButton>
        </div>
      </UiCard>

      <div class="space-y-1.5">
        <button
          v-for="doc in filteredDocs"
          :key="doc.name"
          type="button"
          :data-testid="`docs-front-${doc.name}`"
          class="flex w-full items-center justify-between rounded-lg border px-3 py-2 text-left text-xs transition-colors"
          :class="
            activeDoc?.name === doc.name
              ? 'border-slate-900 bg-slate-900 text-white'
              : 'border-slate-200 bg-white text-slate-700 hover:bg-slate-50'
          "
          @click="selectedName = doc.name"
        >
          <div class="min-w-0">
            <div class="truncate font-mono font-semibold">{{ doc.name }}</div>
            <div
              class="truncate text-[11px]"
              :class="activeDoc?.name === doc.name ? 'text-slate-300' : 'text-slate-500'"
            >
              {{ doc.protocol }} · :{{ doc.port }}
            </div>
          </div>
          <UiBadge :tone="doc.enabled ? 'success' : 'neutral'">
            {{ doc.enabled ? "运行中" : "已停用" }}
          </UiBadge>
        </button>
      </div>
    </aside>

    <section v-if="activeDoc" class="space-y-4" data-testid="docs-detail-pane">
      <UiCard>
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div class="flex flex-wrap items-center gap-2">
              <h1 class="font-mono text-lg font-semibold text-slate-900">
                {{ activeDoc.name }}
              </h1>
              <UiBadge tone="info">{{ activeDoc.protocol }}</UiBadge>
              <UiBadge :tone="activeDoc.enabled ? 'success' : 'neutral'">
                {{ activeDoc.enabled ? "运行中" : "已停用" }}
              </UiBadge>
            </div>
            <p class="mt-1.5 text-sm text-slate-700">{{ activeDoc.summary }}</p>
            <p class="mt-1 text-xs text-slate-500">
              监听 <code class="font-mono text-slate-700">{{ activeDoc.host }}:{{ activeDoc.port }}</code>
              <span v-if="activeDoc.auth_notes"> · 鉴权 {{ activeDoc.auth_notes }}</span>
            </p>
          </div>
        </div>
      </UiCard>

      <UiCard
        v-for="(ep, idx) in activeDoc.endpoints"
        :key="`${ep.method}-${ep.path}-${idx}`"
        :test-id="`docs-endpoint-${idx}`"
      >
        <div class="space-y-3">
          <div class="flex flex-wrap items-center gap-2">
            <UiBadge tone="info">{{ ep.method }}</UiBadge>
            <code class="font-mono text-sm font-semibold text-slate-900">
              {{ ep.path }}
            </code>
            <span class="text-xs text-slate-600">{{ ep.description }}</span>
          </div>

          <div v-if="ep.request_example" class="space-y-1">
            <div class="flex items-center justify-between">
              <span class="text-xs font-medium text-slate-600">请求示例</span>
              <UiButton size="sm" variant="ghost" @click="copyCode(ep.request_example)">
                复制
              </UiButton>
            </div>
            <pre class="overflow-x-auto rounded-lg bg-slate-900 p-3 font-mono text-xs text-slate-100">{{ ep.request_example }}</pre>
          </div>

          <div v-if="ep.response_example" class="space-y-1">
            <div class="flex items-center justify-between">
              <span class="text-xs font-medium text-slate-600">响应示例</span>
              <UiButton size="sm" variant="ghost" @click="copyCode(ep.response_example)">
                复制
              </UiButton>
            </div>
            <pre class="overflow-x-auto rounded-lg bg-slate-900 p-3 font-mono text-xs text-slate-100">{{ ep.response_example }}</pre>
          </div>

          <div v-if="ep.curl_example" class="space-y-1">
            <div class="flex items-center justify-between">
              <span class="text-xs font-medium text-slate-600">调用命令</span>
              <UiButton
                size="sm"
                variant="ghost"
                :test-id="`copy-curl-${idx}`"
                @click="copyCode(ep.curl_example)"
              >
                复制
              </UiButton>
            </div>
            <pre class="overflow-x-auto rounded-lg bg-slate-900 p-3 font-mono text-xs text-emerald-300">{{ ep.curl_example }}</pre>
          </div>
        </div>
      </UiCard>
    </section>

    <UiCard v-else>
      <div class="py-10 text-center text-sm text-slate-500">暂无文档记录</div>
    </UiCard>
  </div>
</template>
