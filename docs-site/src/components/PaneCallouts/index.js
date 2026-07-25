import React from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';
import styles from './styles.module.css';

/**
 * A Context-pane screenshot with a numbered marker over each control and a
 * matching numbered list describing it.
 *
 * The markers are placed in percentages of the image, so they follow it as the
 * image scales; the descriptions stay real text, so they are searchable and
 * readable to a screen reader rather than baked into the PNG.
 *
 * @param {string} src      Site-absolute image path, e.g. `/img/screenshots/panes/move-translate.png`.
 * @param {string} alt      Description of the shot for screen readers.
 * @param {string} [title]  Optional caption above the shot (e.g. which mode it shows).
 * @param {Array}  items    `{x, y, label, children}` per control — `x`/`y` are percentages of the image.
 */
export default function PaneCallouts({src, alt, title, items = []}) {
  return (
    <div className={styles.callouts}>
      <figure className={styles.shot}>
        {title && <figcaption className={styles.caption}>{title}</figcaption>}
        <div className={styles.frame}>
          <img src={useBaseUrl(src)} alt={alt} />
          {items.map((item, i) => (
            <span
              key={i}
              className={styles.marker}
              style={{left: `${item.x}%`, top: `${item.y}%`}}
              aria-hidden="true">
              {i + 1}
            </span>
          ))}
        </div>
      </figure>
      <ol className={styles.legend}>
        {items.map((item, i) => (
          <li key={i}>
            <strong>{item.label}</strong>
            {item.children ? <> — {item.children}</> : null}
          </li>
        ))}
      </ol>
    </div>
  );
}
