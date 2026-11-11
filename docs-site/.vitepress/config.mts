import { defineConfig } from 'vitepress';

export default defineConfig({
  title: 'SynthPanel',
  description: 'Free, open-source LEMP control panel for single Linux servers',
  cleanUrls: true,
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/installation' },
      { text: 'Reference', link: '/reference/cli' },
    ],
    sidebar: {
      '/guide/': [
        {
          text: 'Guide',
          items: [
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quickstart', link: '/guide/quickstart' },
            { text: 'Backups & restore', link: '/guide/backups' },
            { text: 'Updating', link: '/guide/updating' },
            { text: 'Deployments', link: '/guide/deployments' },
            { text: 'Alerts', link: '/guide/alerts' },
            { text: 'Terminal', link: '/guide/terminal' },
            { text: 'PHP', link: '/guide/php' },
            { text: 'Panel certificate', link: '/guide/panel-certificate' },
            { text: 'Troubleshooting', link: '/guide/troubleshooting' },
          ],
        },
      ],
      '/reference/': [
        {
          text: 'Reference',
          items: [
            { text: 'CLI', link: '/reference/cli' },
            { text: 'REST API', link: '/reference/api' },
            { text: 'Security model', link: '/reference/security' },
          ],
        },
      ],
    },
    socialLinks: [{ icon: 'github', link: 'https://github.com/binkapS/synthpanel' }],
    footer: {
      message: 'Released under the AGPL-3.0-or-later license.',
      copyright: 'Copyright © SynthPanel contributors',
    },
  },
});
