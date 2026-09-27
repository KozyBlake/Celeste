package net.sybyline.scarlet.util;

import java.awt.Color;
import java.util.List;

/**
 * VRChat trust rank, inferred from the {@code system_trust_*} / troll entries in
 * a user's {@code tags}. VRChat does not expose trust rank as a first-class API
 * field; it is derived from the highest trust tag present, and a user carrying no
 * trust tags at all is a Visitor.
 * <p>
 * Note: VRChat frequently returns an incomplete tag set for users other than the
 * current user, so this is a best-effort classification. A {@code null} tag list
 * (the lookup gave us nothing) is reported as {@link #UNKNOWN} rather than being
 * mistaken for a Visitor.
 * <p>
 * Ordinal order is low-to-high urgency for the moderator's sake: {@link #NUISANCE}
 * sorts first so it is easy to surface, then Visitor upward, with {@link #UNKNOWN}
 * last.
 */
public enum TrustRank implements HasForegroundColor
{
    // Colours are VRChat's official trust-rank codes (as surfaced by VRCX).
    NUISANCE ("Nuisance",  new Color(0xB8, 0x4A, 0x4A)), // lightened from VRChat #782F2F for readability on the dark table
    VISITOR  ("Visitor",   new Color(0xCC, 0xCC, 0xCC)),
    NEW_USER ("New User",  new Color(0x17, 0x78, 0xFF)),
    USER     ("User",      new Color(0x2B, 0xCF, 0x5C)),
    KNOWN    ("Known",     new Color(0xFF, 0x7B, 0x42)),
    TRUSTED  ("Trusted",   new Color(0x81, 0x43, 0xE6)),
    UNKNOWN  ("Unknown",   null),
    ;

    private final String label;
    /** Indicative colour for this rank (VRChat-like); {@code null} for {@link #UNKNOWN}. */
    public final Color color;

    TrustRank(String label, Color color)
    {
        this.label = label;
        this.color = color;
    }

    public boolean isNuisance()
    {
        return this == NUISANCE;
    }

    public boolean isVisitor()
    {
        return this == VISITOR;
    }

    @Override
    public Color foregroundColor()
    {
        return this.color;
    }

    @Override
    public String toString()
    {
        return this.label;
    }

    /**
     * Classify a trust rank from a user's tag list. A {@code null} list yields
     * {@link #UNKNOWN}; a non-null list carrying no trust tag yields
     * {@link #VISITOR}. A troll tag overrides any trust tag.
     */
    public static TrustRank of(List<String> tags)
    {
        if (tags == null)
            return UNKNOWN;
        if (tags.contains("system_troll") || tags.contains("system_probable_troll"))
            return NUISANCE;
        if (tags.contains("system_trust_veteran"))
            return TRUSTED;
        if (tags.contains("system_trust_trusted"))
            return KNOWN;
        if (tags.contains("system_trust_known"))
            return USER;
        if (tags.contains("system_trust_basic"))
            return NEW_USER;
        return VISITOR;
    }
}
