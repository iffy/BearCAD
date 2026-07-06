import clsx from 'clsx';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

const FeatureList = [
  {
    title: 'Quickstart',
    to: '/docs/quickstart',
    description: (
      <>
        Build a real part in about ten minutes: sketch a bracket freehand, square it up with
        constraints, drive the bend angle with a parameter, and export it for 3D printing.
      </>
    ),
  },
  {
    title: 'Tools & Navigation',
    to: '/docs/tools',
    description: (
      <>
        Tool-by-tool reference for Select, Sketch, Rectangle, Line, Circle, Fillet, Chamfer,
        Construction Plane, Extrude, Revolve, Dimension, and Constraint — plus orbit/pan/zoom,
        the view bear, and sketch mode.
      </>
    ),
  },
  {
    title: 'Scripting',
    to: '/docs/scripting',
    description: (
      <>
        Drive BearCAD from Lua: declarative <code>bearcad.*</code> modeling and the{' '}
        <code>bearcad.ui.*</code> interaction namespace. The same action layer powers the GUI,
        the command palette, and scripts — anything you can click, you can script.
      </>
    ),
  },
];

function Feature({title, to, description}) {
  return (
    <div className={clsx('col col--4')}>
      <div className="text--center padding-horiz--md">
        <Heading as="h3">
          <Link to={to}>{title}</Link>
        </Heading>
        <p>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures() {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
