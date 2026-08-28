import type { Metadata } from 'next';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';

import './global.css';

const description = 'AI detection and plagiarism checks from the terminal.';
const socialImage = '/landing/images/og-image.jpg';

export const metadata: Metadata = {
  metadataBase: new URL('https://pangram.micr.dev'),
  title: { default: 'Pangram CLI', template: '%s | Pangram CLI' },
  description,
  icons: { icon: '/landing/images/pangram-cli-logo.svg' },
  openGraph: {
    type: 'website',
    title: 'Pangram CLI',
    description,
    siteName: 'pangram-cli',
    images: [{
      url: socialImage,
      width: 1200,
      height: 630,
      alt: 'pangram-cli - AI detection without leaving your terminal',
    }],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Pangram CLI',
    description,
    images: [socialImage],
  },
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
