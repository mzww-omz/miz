CREATE OR REPLACE FUNCTION miz_post_visible(viewer BYTEA, target_post BYTEA) RETURNS BOOLEAN
LANGUAGE sql STABLE PARALLEL SAFE AS $$
  WITH RECURSIVE post_chain AS (
    SELECT id, author_id, reply_to_post_id, state, effective_visibility FROM posts WHERE id = target_post
    UNION ALL
    SELECT parent.id, parent.author_id, parent.reply_to_post_id, parent.state, parent.effective_visibility
    FROM posts parent JOIN post_chain child ON parent.id = child.reply_to_post_id
  )
  SELECT EXISTS (SELECT 1 FROM post_chain WHERE id = target_post)
    AND NOT EXISTS (
      SELECT 1 FROM post_chain post
      WHERE post.state NOT IN ('published', 'tombstone')
         OR NOT (
           (post.state = 'tombstone' AND post.author_id = decode(repeat('00', 16), 'hex'))
           OR miz_profile_visible(viewer, post.author_id)
         )
         OR (post.reply_to_post_id IS NULL
             AND post.effective_visibility = 'followers'
             AND viewer <> post.author_id
             AND NOT EXISTS (
               SELECT 1 FROM follow_relationships relationship
               WHERE relationship.follower_id = viewer
                 AND relationship.followee_id = post.author_id
                 AND relationship.status = 'accepted'
             ))
    )
$$;
