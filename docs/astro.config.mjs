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
