package net.sybyline.scarlet.util;

import java.awt.Color;

/**
 * A cell value that carries its own preferred foreground colour for the props
 * table. When a rendered value implements this and returns a non-null colour,
 * that colour is used for <em>that cell only</em> — it takes precedence over the
 * row-level advisory colour, and it does not affect any other column.
 */
public interface HasForegroundColor
{
    /** @return the foreground colour for this value, or {@code null} to use the default. */
    Color foregroundColor();
}
