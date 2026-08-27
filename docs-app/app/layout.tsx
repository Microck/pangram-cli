import type { Metadata } from 'next';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';

import './global.css';

export const metadata: Metadata = {
  title: { default: 'Pangram CLI', template: '%s | Pangram CLI' },
  description: 'AI detection and plagiarism checks from the terminal.',
  icons: { icon: '/landing/images/pangram-cli-logo.svg' },
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" style={{ colorScheme: 'light' }} suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <RootProvider theme={{ defaultTheme: 'light', enableSystem: false }}>{children}</RootProvider>
      </body>
    </html>
  );
}
