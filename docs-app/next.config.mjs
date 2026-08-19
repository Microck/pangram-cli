import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  /*
   * Keep the supplied landing page independent from the Fumadocs React tree.
   * A rewrite preserves the root URL while /docs remains a normal Next route.
   */
  async rewrites() {
    return [{ source: '/', destination: '/landing.html' }];
  },
  async redirects() {
    return [{
      source: '/install',
      destination: 'https://github.com/Microck/pangram-cli/releases/latest/download/pangram-installer.sh',
      permanent: false,
    }];
  },
};

export default withMDX(config);
