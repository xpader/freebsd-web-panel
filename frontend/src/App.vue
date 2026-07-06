<script setup>
import { onMounted } from 'vue';
import { useAuthStore } from './stores/auth.js';

const auth = useAuthStore();

onMounted(() => {
  if (auth.isLoggedIn()) auth.fetchUser();
});
</script>

<template>
  <router-view />
  <ToastContainer />
  <DialogHost />
</template>

<script>
import ToastContainer from './components/ui/ToastContainer.vue';
import DialogHost from './components/ui/DialogHost.vue';
export default { components: { ToastContainer, DialogHost } };
</script>
