<script setup lang="ts">
import { ref } from "vue";
import Sandbox from "./components/Sandbox.vue";
import Pipeline from "./components/Pipeline.vue";

const tab = ref<"world" | "pipeline">("world");
</script>

<template>
  <div class="app-shell">
    <nav class="tabs">
      <button :class="{ active: tab === 'world' }" @click="tab = 'world'">🌍 World</button>
      <button :class="{ active: tab === 'pipeline' }" @click="tab = 'pipeline'">🔁 Pipeline</button>
    </nav>
    <div class="page">
      <Sandbox v-if="tab === 'world'" />
      <Pipeline v-else />
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.tabs {
  display: flex;
  gap: 8px;
  padding: 10px 16px;
  border-bottom: 1px solid #ffffff22;
}
.page {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.page > * {
  flex: 1;
  min-height: 0;
}
</style>
