<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import { createApp, deleteApp, fetchApps, updateApp } from "../../api/assets";
import { ApiRequestError } from "../../api/http";
import type { AppMeta, AppUpsertPayload, BusinessMethodId } from "../../api/types";
import { useToast } from "../../composables/useToast";
import UiBadge from "../ui/UiBadge.vue";
import UiButton from "../ui/UiButton.vue";
import UiCard from "../ui/UiCard.vue";
import UiDialog from "../ui/UiDialog.vue";
import UiInput from "../ui/UiInput.vue";

const { push } = useToast();

const ALL_METHODS: ReadonlyArray<BusinessMethodId> = [
  "x_upt.posts.search",
  "x_upt.photos.render",
  "x_upt.imgdef.compare",
];

const loading = ref(false);
const apps = ref<AppMeta[]>([]);

const dialogOpen = ref(false);
const saving = ref(false);
const isEditing = ref(false);
const form = reactive({
  client_id: "",
  client_secret: "",
  name: "",
  rsa_public_pem: "",
  methods: [...ALL_METHODS] as string[],
});

async function loadApps(): Promise<void> {
  loading.value = true;
  try {
    const res = await fetchApps();
    apps.value = res.items;
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    loading.value = false;
  }
}

function openCreate(): void {
  isEditing.value = false;
  form.client_id = "";
  form.client_secret = "";
  form.name = "";
  form.rsa_public_pem = "";
  form.methods = [...ALL_METHODS];
  dialogOpen.value = true;
}

function openEdit(app: AppMeta): void {
  isEditing.value = true;
  form.client_id = app.client_id;
  form.client_secret = "";
  form.name = app.name;
  form.rsa_public_pem = "";
  form.methods = [...app.methods];
  dialogOpen.value = true;
}

function toggleMethod(method: string): void {
  if (form.methods.includes(method)) {
    form.methods = form.methods.filter((m) => m !== method);
  } else {
    form.methods = [...form.methods, method];
  }
}

async function handleSave(): Promise<void> {
  saving.value = true;
  try {
    const payload: AppUpsertPayload = {
      client_id: form.client_id.trim(),
      client_secret: form.client_secret,
      name: form.name.trim() || form.client_id.trim(),
      methods: [...form.methods],
      rsa_public_pem:
        form.rsa_public_pem.trim().length > 0 ? form.rsa_public_pem.trim() : null,
    };
    if (isEditing.value) {
      await updateApp(payload.client_id, payload);
    } else {
      await createApp(payload);
    }
    dialogOpen.value = false;
    push("已保存", "success");
    await loadApps();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "保存失败", "error");
  } finally {
    saving.value = false;
  }
}

async function handleDelete(clientId: string): Promise<void> {
  try {
    await deleteApp(clientId);
    push("已删除", "info");
    await loadApps();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "删除失败", "error");
  }
}

onMounted(() => {
  void loadApps();
});
</script>

<template>
  <div class="space-y-4">
    <UiCard title="应用凭据">
      <template #actions>
        <UiButton size="sm" :loading="loading" test-id="app-refresh-btn" @click="loadApps">
          刷新
        </UiButton>
        <UiButton size="sm" variant="primary" test-id="app-create-open" @click="openCreate">
          新建
        </UiButton>
      </template>

      <div v-if="apps.length === 0" class="py-10 text-center text-sm text-slate-500">
        暂无应用凭据
      </div>

      <div v-else class="overflow-x-auto">
        <table class="min-w-full divide-y divide-slate-200 text-left text-xs">
          <thead class="bg-slate-50 text-slate-600">
            <tr>
              <th class="px-3 py-2 font-medium">客户端 ID</th>
              <th class="px-3 py-2 font-medium">名称</th>
              <th class="px-3 py-2 font-medium">密钥摘要</th>
              <th class="px-3 py-2 font-medium">RSA 公钥</th>
              <th class="px-3 py-2 font-medium">授权接口</th>
              <th class="px-3 py-2 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="app in apps" :key="app.client_id" :data-testid="`app-row-${app.client_id}`">
              <td class="px-3 py-2 font-mono font-medium text-slate-900">
                {{ app.client_id }}
              </td>
              <td class="px-3 py-2 text-slate-700">{{ app.name }}</td>
              <td class="px-3 py-2 font-mono text-slate-500">
                {{ app.secret_preview }}
              </td>
              <td class="px-3 py-2">
                <UiBadge :tone="app.has_rsa_public_pem ? 'success' : 'neutral'">
                  {{ app.has_rsa_public_pem ? "已配置" : "无" }}
                </UiBadge>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <UiBadge v-for="m in app.methods" :key="m" tone="info">
                    {{ m }}
                  </UiBadge>
                </div>
              </td>
              <td class="px-3 py-2 text-right">
                <div class="inline-flex items-center gap-1.5">
                  <UiButton
                    size="sm"
                    variant="ghost"
                    :test-id="`app-edit-${app.client_id}`"
                    @click="openEdit(app)"
                  >
                    编辑
                  </UiButton>
                  <UiButton
                    size="sm"
                    variant="danger"
                    :test-id="`app-delete-${app.client_id}`"
                    @click="handleDelete(app.client_id)"
                  >
                    删除
                  </UiButton>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </UiCard>

    <UiDialog
      v-model:open="dialogOpen"
      :title="isEditing ? `编辑凭据 ${form.client_id}` : '新建应用凭据'"
    >
      <form class="space-y-3" @submit.prevent="handleSave">
        <div class="grid gap-3 sm:grid-cols-2">
          <UiInput
            v-model="form.client_id"
            :disabled="isEditing"
            label="客户端 ID"
            placeholder="demo_client"
            test-id="app-client-id"
          />
          <UiInput
            v-model="form.name"
            label="应用名称"
            placeholder="Demo Caller"
            test-id="app-name"
          />
        </div>
        <UiInput
          v-model="form.client_secret"
          type="password"
          label="客户端密钥 (8–128 字节)"
          placeholder="secret_12345678"
          test-id="app-client-secret"
        />
        <div class="space-y-1.5">
          <span class="block text-xs font-medium text-slate-700">授权接口</span>
          <div class="flex flex-wrap gap-2">
            <label
              v-for="m in ALL_METHODS"
              :key="m"
              class="inline-flex cursor-pointer items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs"
              :class="
                form.methods.includes(m)
                  ? 'border-slate-900 bg-slate-900 text-white'
                  : 'border-slate-200 bg-white text-slate-700'
              "
            >
              <input
                type="checkbox"
                class="sr-only"
                :checked="form.methods.includes(m)"
                @change="toggleMethod(m)"
              />
              <span class="font-mono">{{ m }}</span>
            </label>
          </div>
        </div>
        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">
            RSA 公钥 PEM (可选，用于 JWT RS*)
          </span>
          <textarea
            v-model="form.rsa_public_pem"
            rows="4"
            placeholder="-----BEGIN PUBLIC KEY-----"
            data-testid="app-rsa-pem"
            class="block w-full rounded-lg border border-slate-200 p-2 font-mono text-xs focus:border-slate-900 focus:outline-none"
          />
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <UiButton @click="dialogOpen = false">取消</UiButton>
          <UiButton
            type="submit"
            variant="primary"
            :loading="saving"
            test-id="app-save-btn"
          >
            保存
          </UiButton>
        </div>
      </form>
    </UiDialog>
  </div>
</template>
