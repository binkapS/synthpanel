<script lang="ts">
  import { currentPath } from '../lib/router';
  import SiteDetail from './SiteDetail.svelte';

  const id = $derived($currentPath.replace('/sites/', ''));
</script>

<SiteDetail {id} />
