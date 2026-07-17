<script setup>
// User info is loaded globally in the router guard and shared via Pinia.
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
