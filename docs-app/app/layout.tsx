import type { Metadata } from 'next';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';

import './global.css';

export const metadata: Metadata = {
  title: { default: 'Pangram CLI', template: '%s | Pangram CLI' },
  description: 'AI detection and plagiarism checks from the terminal.',
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className="dark" style={{ colorScheme: 'dark' }} suppressHydrationWarning>
      <body className="flex min-h-screen flex-col bg-black text-white">
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
