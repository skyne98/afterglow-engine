import { createApp } from 'vue';
import PaintApp from './PaintApp.vue';
import './paint-style.css';

createApp(PaintApp).mount('#app');
await import('./paint-demo.ts');
