import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import HomepageFeatures from '@site/src/components/HomepageFeatures';

import Heading from '@theme/Heading';
import styles from './index.module.css';

// Latest-release asset URLs live on the /docs/downloads page (same names as
// src/release_artifacts.rs); the navbar and hero only link there.
// The hosted web build (wasm), deployed alongside the docs by CI. Chromebooks
// install this as a PWA (Install app in Chrome); same path as "Run in browser".
const WEB_APP_PATH = 'pathname:///app/';
// Choose what you want to pay (Stripe payment link).
const PAY_URL = 'https://buy.stripe.com/4gMbJ39g2gsH4hKd9cdQQ00';
// The other three of the four main actions, besides the prominent "Run in your
// browser" CTA. Download points at a dedicated page so the navbar can reuse it.
// `to` (not `href`) marks an internal route so it navigates without a reload.
const ACTIONS = [
  {label: '▶  Run in your browser', href: WEB_APP_PATH, primary: true},
  {label: 'Read the docs', to: '/docs/intro'},
  {label: 'Choose what to pay', href: PAY_URL},
  {label: 'Download', to: '/docs/downloads'},
];

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <img
          className={styles.heroLogo}
          src={useBaseUrl('/img/logo.png')}
          alt="BearCAD bear icon"
          width="160"
          height="160"
        />
        <Heading as="h1" className="hero__title">
          {siteConfig.title}
        </Heading>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.ctaRow}>
          <Link
            className={clsx('button button--lg', styles.ctaButton)}
            href={ACTIONS[0].href}>
            {ACTIONS[0].label}
          </Link>
        </div>
        <div className={styles.buttons}>
          {ACTIONS.slice(1).map(({label, href, to}) => (
            <Link
              key={label}
              className={clsx('button button--lg', styles.downloadButton)}
              {...(to ? {to} : {href})}>
              {label}
            </Link>
          ))}
        </div>
      </div>
    </header>
  );
}

function HomepageScreenshot() {
  return (
    <section className={styles.screenshotSection}>
      <div className="container">
        <img
          className={styles.screenshot}
          src={useBaseUrl('/img/screenshots/quickstart.png')}
          alt="BearCAD editing the Quickstart's 120-degree bracket: rounded bend, countersunk screw holes"
        />
        <p className={styles.screenshotCaption}>
          The <Link to="/docs/quickstart">Quickstart</Link> bracket — sketched freehand, squared
          up by the constraint solver, rebuilt from parameters.
        </p>
      </div>
    </section>
  );
}

export default function Home() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title={siteConfig.title}
      description="BearCAD — local-first, parametric CAD with a shared GUI and Lua scripting action layer.">
      <HomepageHeader />
      <main>
        <HomepageScreenshot />
        <HomepageFeatures />
      </main>
    </Layout>
  );
}
