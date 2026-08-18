import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import GetBearCAD from '@site/src/components/GetBearCAD';
import {WEB_APP_PATH} from '@site/src/site';

import styles from './index.module.css';

const STATS = [
  {value: '~ 50 MB', label: 'The whole app'},
  {value: '~ 0.5s', label: 'Cold launch'},
  {value: 'No account', label: 'Files stay yours'},
  {value: 'STL / STEP / 3MF', label: 'Print or export'},
];

const TRAITS = [
  {
    title: 'Sketch sloppy',
    body: 'Draw a bracket freehand. The solver squares it up. Change a parameter later and the part rebuilds.',
  },
  {
    title: 'Tiny on purpose',
    body: 'A single executable with a real BREP kernel. No splash screen, no sign-in, no 8 GB installer.',
  },
  {
    title: 'Click it, script it',
    body: 'The same actions drive the GUI, the command palette, and Lua. If you can click it, you can script it.',
  },
];

const MORE = [
  {label: 'Quickstart', to: '/docs/quickstart'},
  {label: 'Why BearCAD?', to: '/docs/why'},
  {label: 'Tools', to: '/docs/tools'},
  {label: 'Scripting', to: '/docs/scripting'},
];

function Hero() {
  const {siteConfig} = useDocusaurusContext();
  const version = siteConfig.customFields?.appVersion ?? '';
  return (
    <header className={styles.hero}>
      <div className={styles.heroInner}>
        <div className={styles.heroBrand}>
          <img
            className={styles.logo}
            src={useBaseUrl('/img/logo.png')}
            alt=""
            width="220"
            height="220"
          />
        </div>
        <div className={styles.heroCopy}>
          <Heading as="h1" className={styles.title}>
            <span className={styles.titleSoft}>Small CAD</span>
            <br />
            <span className={styles.titleSoft}>Quick CAD</span>
            <br />
            <span className={styles.titleSoft}>Fun CAD</span>
            <br />
            <span className={styles.titleName}>
              BearCAD
            </span>
          </Heading>
          <p className={styles.sub}>
            Sketch, extrude, revolve, import/export, constrain and 3D print. Half-second launch. No
            account.
          </p>
        </div>
        <div className={styles.heroCtas}>
          <Link className={clsx(styles.btn, styles.btnPrimary)} href={WEB_APP_PATH}>
            Open in your browser
          </Link>
          <Link className={clsx(styles.btn, styles.btnGhost)} href="#get">
            Download {version ? <>v{version}</> : null}
          </Link>
        </div>
      </div>
      <div className={styles.shotWrap}>
        <img
          className={styles.shot}
          src={useBaseUrl('/img/screenshots/materials.png')}
          alt="Nine cubes in a 2×2×2 with a centre cube, each a different material colour, some with a hole, sphere bite, chamfer or fillet"
        />
      </div>
    </header>
  );
}

function Stats() {
  return (
    <section className={styles.stats} aria-label="At a glance">
      <div className={styles.statsGrid}>
        {STATS.map(({value, label}) => (
          <div key={value} className={styles.stat}>
            <div className={styles.statValue}>{value}</div>
            <div className={styles.statLabel}>{label}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Get() {
  return (
    <section className={styles.get}>
      <div className={styles.narrow}>
        <Heading as="h2" className={styles.sectionTitle} id="get">
          Get BearCAD
        </Heading>
        <GetBearCAD />
      </div>
    </section>
  );
}

function Traits() {
  return (
    <section className={styles.traits}>
      <div className={styles.narrow}>
        <div className={styles.traitGrid}>
          {TRAITS.map(({title, body}) => (
            <div key={title} className={styles.trait}>
              <Heading as="h3" className={styles.traitTitle}>
                {title}
              </Heading>
              <p className={styles.traitBody}>{body}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function More() {
  return (
    <section className={styles.more}>
      <div className={styles.narrow}>
        <p className={styles.moreLabel}>Docs, if you want them</p>
        <ul className={styles.moreLinks}>
          {MORE.map(({label, to}) => (
            <li key={to}>
              <Link to={to}>{label}</Link>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

export default function Home() {
  return (
    <Layout
      title="Small, quick, fun CAD"
      description="A tiny parametric CAD app. Half-second launch, no account, name your price."
      wrapperClassName={styles.page}>
      <main>
        <Hero />
        <Stats />
        <Get />
        <Traits />
        <More />
      </main>
    </Layout>
  );
}
