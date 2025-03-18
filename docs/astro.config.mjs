import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import cloudflare from '@astrojs/cloudflare';

// https://astro.build/config
export default defineConfig({
  output: 'server',
  adapter: cloudflare(),
  integrations: [
    starlight({
      title: 'Zhang Accounting v0.1.10',
      head: [{
        tag: 'script',
        attrs: {
          src: 'https://analytics.kilerd.me/script.js',
          'data-website-id': '3648c073-f066-4ddd-b07f-a1e208505a42',
          defer: true,
        },
      }],
      social: {
        github: 'https://github.com/zhang-accounting/zhang',
        discord: 'https://discord.gg/mcZpvmFeRV',
      },
      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        'zh-cn': {
          label: '简体中文',
          lang: 'zh-CN',
        },
      },
      sidebar: [
        {
          label: 'Installation',
          autogenerate: { directory: 'installation' },
        },
        {
          label: 'Datasource',
          autogenerate: { directory: 'datasources' },
        },
        {
          label: 'Directives',
          autogenerate: { directory: 'directives' },
        },
        {
          label: 'User Guides',
          autogenerate: { directory: 'user-guide' },
        },
        {
          label: 'Developer Guides',
          autogenerate: { directory: 'developer-guides' },
        },
      ],
    }),
  ],
});
