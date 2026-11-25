use crate::failure::AuthFailure;
use url::Url;

pub const MAX_REDIRECTS: usize = 10;

/// Port of `Test-AllowedUri`: https, default port, no userinfo, host on the allow-list (case-insensitive).
pub fn is_allowed_uri(u: &Url, allowed_hosts: &[&str]) -> bool {
    u.scheme() == "https"
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
        && u.host_str()
            .is_some_and(|h| allowed_hosts.iter().any(|a| a.eq_ignore_ascii_case(h)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectDecision {
    /// Not a redirect: stop following and read the page.
    Done,
    /// Follow with a GET to this URL.
    Follow(Url),
}

/// Decide what to do with a response. Port of the redirect loop: only 301/302/303 are followed, only with GET
/// (a 307/308 would replay the password POST, so it is refused), the `Location` must resolve against the current
/// URL to an allow-listed https URL, and `hops_so_far` must stay below [`MAX_REDIRECTS`].
pub fn follow_redirect(
    status: u16,
    location: Option<&str>,
    base: &Url,
    allowed_hosts: &[&str],
    hops_so_far: usize,
) -> Result<RedirectDecision, AuthFailure> {
    if !(300..400).contains(&status) {
        return Ok(RedirectDecision::Done);
    }
    if hops_so_far >= MAX_REDIRECTS {
        return Err(AuthFailure::RedirectLimit);
    }
    if !matches!(status, 301..=303) {
        return Err(AuthFailure::RedirectRefused);
    }
    let location = location.ok_or(AuthFailure::UnexpectedPage)?;
    let next = base
        .join(location)
        .map_err(|_| AuthFailure::UnexpectedPage)?;
    if !is_allowed_uri(&next, allowed_hosts) {
        return Err(AuthFailure::HostNotAllowed);
    }
    Ok(RedirectDecision::Follow(next))
}
