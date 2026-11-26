<script setup lang="ts">
import { onMounted, reactive, ref, watch } from "vue";

import {
  deleteEnvelopeSecret,
  fetchEnvelopeProfiles,
  fetchEnvelopeSecrets,
  replaceEnvelopeProfiles,
  upsertEnvelopeSecret,
} from "../../api/envelopes";
import { ApiRequestError } from "../../api/http";
import type { SecretMeta, SecretUpsertPayload } from "../../api/types";
import { useToast } from "../../composables/useToast";
import UiBadge from "../ui/UiBadge.vue";
import UiButton from "../ui/UiButton.vue";
import UiCard from "../ui/UiCard.vue";
import UiInput from "../ui/UiInput.vue";
import UiSelect from "../ui/UiSelect.vue";

const props = defineProps<{
  initialFront?: string;
}>();

const { push } = useToast();

const envelopeFrontOptions = [
  { value: "envelope_token_plain", label: "envelope_token_plain" },
  { value: "envelope_token_crypt", label: "envelope_token_crypt" },
  { value: "envelope_sign_plain", label: "envelope_sign_plain" },
  { value: "envelope_sign_crypt", label: "envelope_sign_crypt" },
  { value: "envelope_cred_plain", label: "envelope_cred_plain" },
  { value: "envelope_cred_crypt", label: "envelope_cred_crypt" },
] as const;

const modeOptions = [
  { value: "generate", label: "随机生成" },
  { value: "manual", label: "手动录入" },
] as const;

const generatePresets = [
  { value: "aes_128", label: "AES-128 (16B)" },
  { value: "aes_256", label: "AES-256 (32B)" },
  { value: "sm4", label: "SM4 (16B)" },
  { value: "des", label: "DES (8B)" },
  { value: "3des", label: "3DES (24B)" },
  { value: "hmac_sha256", label: "HMAC (32B)" },
  { value: "rsa_2048", label: "RSA-2048" },
  { value: "sm2", label: "SM2" },
  { value: "text", label: "随机口令文本" },
] as const;

const secretKindOptions = [
  { value: "text", label: "UTF-8 文本 (text)" },
  { value: "bytes_hex", label: "十六进制字节 (bytes_hex)" },
  { value: "pem", label: "PEM 密钥 (pem)" },
] as const;

const secretsLoading = ref(false);
const secretSaving = ref(false);
const secrets = ref<SecretMeta[]>([]);

const secretForm = reactive({
  mode: "generate",
  name: "",
  generatePreset: "aes_128",
  generateCustom: "aes_128",
  secretKind: "text",
  value: "",
});

const selectedFront = ref(
  props.initialFront &&
    envelopeFrontOptions.some((o) => o.value === props.initialFront)
    ? props.initialFront
    : "envelope_token_plain",
);
const profilesLoading = ref(false);
const profilesSaving = ref(false);
const profilesJson = ref("[]");

watch(
  () => secretForm.generatePreset,
  (val) => {
    secretForm.generateCustom = val;
  },
);

watch(selectedFront, () => {
  void loadProfiles();
});

async function loadSecrets(): Promise<void> {
  secretsLoading.value = true;
  try {
    const res = await fetchEnvelopeSecrets();
    secrets.value = res.items;
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    secretsLoading.value = false;
  }
}

async function handleSaveSecret(): Promise<void> {
  const name = secretForm.name.trim();
  if (!name) {
    push("密钥名称为空", "error");
    return;
  }
  secretSaving.value = true;
  try {
    const payload: SecretUpsertPayload =
      secretForm.mode === "generate"
        ? {
            name,
            generate: secretForm.generateCustom.trim() || secretForm.generatePreset,
          }
        : {
            name,
            secret_kind: secretForm.secretKind,
            value: secretForm.value,
          };
    await upsertEnvelopeSecret(payload);
    secretForm.name = "";
    secretForm.value = "";
    push("已保存", "success");
    await loadSecrets();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "保存失败", "error");
  } finally {
    secretSaving.value = false;
  }
}

async function handleDeleteSecret(name: string): Promise<void> {
  try {
    await deleteEnvelopeSecret(name);
    push("已删除", "info");
    await loadSecrets();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "删除失败", "error");
  }
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    push("已复制", "success");
  } catch {
    push("复制失败", "error");
  }
}

async function loadProfiles(): Promise<void> {
  profilesLoading.value = true;
  try {
    const res = await fetchEnvelopeProfiles(selectedFront.value);
    profilesJson.value = JSON.stringify(res.profiles, null, 2);
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "画像读取失败", "error");
  } finally {
    profilesLoading.value = false;
  }
}

async function handleSaveProfiles(): Promise<void> {
  profilesSaving.value = true;
  try {
    const parsed: unknown = JSON.parse(profilesJson.value);
    if (!Array.isArray(parsed)) {
      push("画像须为 JSON 数组", "error");
      return;
    }
    const res = await replaceEnvelopeProfiles(selectedFront.value, parsed);
    profilesJson.value = JSON.stringify(res.profiles, null, 2);
    push("已保存", "success");
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "画像保存失败", "error");
  } finally {
    profilesSaving.value = false;
  }
}

onMounted(() => {
  void loadSecrets();
  void loadProfiles();
});
</script>

<template>
  <div class="grid gap-6 lg:grid-cols-2">
    <UiCard title="信封密钥">
      <template #actions>
        <UiButton size="sm" :loading="secretsLoading" @click="loadSecrets">
          刷新
        </UiButton>
      </template>

      <form class="mb-5 space-y-3 rounded-lg border border-slate-200 bg-slate-50/70 p-3.5" @submit.prevent="handleSaveSecret">
        <div class="grid gap-3 sm:grid-cols-2">
          <UiSelect
            v-model="secretForm.mode"
            :options="modeOptions"
            label="录入方式"
            test-id="secret-mode-select"
          />
          <UiInput
            v-model="secretForm.name"
            label="密钥名称"
            placeholder="aes.key"
            test-id="secret-name-input"
          />
        </div>

        <div v-if="secretForm.mode === 'generate'" class="grid gap-3 sm:grid-cols-2">
          <UiSelect
            v-model="secretForm.generatePreset"
            :options="generatePresets"
            label="算法预设"
            test-id="secret-generate-select"
          />
          <UiInput
            v-model="secretForm.generateCustom"
            label="生成参数"
            placeholder="aes_128"
            test-id="secret-generate-input"
          />
        </div>

        <div v-else class="space-y-3">
          <UiSelect
            v-model="secretForm.secretKind"
            :options="secretKindOptions"
            label="密钥类型"
            test-id="secret-kind-select"
          />
          <label class="block">
            <span class="mb-1.5 block text-xs font-medium text-slate-700">密钥内容</span>
            <textarea
              v-model="secretForm.value"
              rows="3"
              placeholder="00112233445566778899aabbccddeeff"
              data-testid="secret-value-input"
              class="block w-full rounded-lg border border-slate-200 bg-white p-2 font-mono text-xs focus:border-slate-900 focus:outline-none"
            />
          </label>
        </div>

        <div class="flex justify-end">
          <UiButton
            type="submit"
            variant="primary"
            size="sm"
            :loading="secretSaving"
            test-id="secret-submit-btn"
          >
            {{ secretForm.mode === "generate" ? "生成密钥" : "保存" }}
          </UiButton>
        </div>
      </form>

      <div v-if="secrets.length === 0" class="py-6 text-center text-xs text-slate-500">
        暂无信封密钥
      </div>

      <div v-else class="overflow-x-auto">
        <table class="min-w-full divide-y divide-slate-200 text-left text-xs">
          <thead class="bg-slate-50 text-slate-600">
            <tr>
              <th class="px-2.5 py-2 font-medium">名称</th>
              <th class="px-2.5 py-2 font-medium">类型</th>
              <th class="px-2.5 py-2 font-medium">长度</th>
              <th class="px-2.5 py-2 font-medium">指纹</th>
              <th class="px-2.5 py-2 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="s in secrets" :key="s.name" :data-testid="`secret-row-${s.name}`">
              <td class="px-2.5 py-2 font-mono font-medium text-slate-900">
                {{ s.name }}
              </td>
              <td class="px-2.5 py-2">
                <UiBadge tone="neutral">{{ s.secret_kind }}</UiBadge>
              </td>
              <td class="px-2.5 py-2 font-mono text-slate-600">{{ s.byte_len }} B</td>
              <td class="max-w-[120px] truncate px-2.5 py-2 font-mono text-slate-500" :title="s.fingerprint">
                {{ s.fingerprint }}
              </td>
              <td class="px-2.5 py-2 text-right">
                <div class="inline-flex items-center gap-1">
                  <UiButton
                    v-if="s.public_material"
                    size="sm"
                    variant="ghost"
                    @click="copyText(s.public_material)"
                  >
                    复制
                  </UiButton>
                  <UiButton
                    size="sm"
                    variant="danger"
                    :test-id="`secret-delete-${s.name}`"
                    @click="handleDeleteSecret(s.name)"
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

    <UiCard title="信封画像">
      <template #actions>
        <div class="flex items-center gap-2">
          <UiButton size="sm" :loading="profilesLoading" @click="loadProfiles">
            刷新
          </UiButton>
          <UiButton
            size="sm"
            variant="primary"
            :loading="profilesSaving"
            test-id="envelope-profiles-save"
            @click="handleSaveProfiles"
          >
            保存
          </UiButton>
        </div>
      </template>

      <div class="space-y-3">
        <UiSelect
          v-model="selectedFront"
          :options="envelopeFrontOptions"
          label="目标信封面"
          test-id="envelope-front-select"
        />

        <label class="block">
          <span class="mb-1.5 block text-xs font-medium text-slate-700">
            画像配置 JSON 数组
          </span>
          <textarea
            v-model="profilesJson"
            rows="16"
            placeholder="[]"
            data-testid="envelope-profiles-editor"
            class="block w-full rounded-lg border border-slate-200 bg-slate-50 p-3 font-mono text-xs leading-relaxed text-slate-900 focus:border-slate-900 focus:bg-white focus:outline-none"
          />
        </label>
      </div>
    </UiCard>
  </div>
</template>
