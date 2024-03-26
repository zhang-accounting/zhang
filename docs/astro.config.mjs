import {defineConfig} from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
    integrations: [
        starlight({
            title: 'Zhang Accounting',
            social: {
                github: 'https://github.com/zhang-accounting/zhang',
            },
            defaultLocale: 'root',
            locales: {
                root: {label: "English", lang: "en"},
                'zh-cn': {
                    label: "简体中文",
                    lang: 'zh-CN'
                }
            },
            sidebar: [
                {
                    label: 'Installation',
                    autogenerate: {directory: 'installation'},
                },
                {
                    label: 'Datasource',
                    autogenerate: {directory: 'datasources'},
                },
                {
                    label: 'Directives',
                    autogenerate: {directory: 'directives'},
                },
                {
                    label: 'Developer Guides',
                    autogenerate: {directory: 'developer-guides'},
                },
            ],
        }),
    ],
});
