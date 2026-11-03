import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
export default defineConfig({plugins:[react()],clearScreen:false,server:{port:1420,strictPort:true},test:{environment:'jsdom',setupFiles:['./frontend/test-setup.ts'],css:false}});
