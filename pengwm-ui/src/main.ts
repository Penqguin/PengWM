/**
 * Entry point for the PengWM UI application.
 * Initializes the Svelte 5 application and mounts it to the DOM.
 */
import { mount } from 'svelte';
import App from './App.svelte';

const app = mount(App, {
	target: document.getElementById('app')!,
});

export default app;
